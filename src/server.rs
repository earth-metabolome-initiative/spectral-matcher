#![cfg(not(target_arch = "wasm32"))]
//! Minimal HTTP server exposing synchronous and asynchronous matcher endpoints.
//!
//! The server intentionally keeps the transport layer small and dependency-light.
//! It accepts JSON requests over plain HTTP, dispatches them to the search and
//! networking pipelines, and stores asynchronous job state in memory.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::api::{
    HealthResponse, JobCreatedResponse, JobProgress, JobProgressStage, JobStatus,
    JobStatusResponse, MatcherJobResult, NetworkRequest, SearchRequest,
};
use crate::search::{
    build_network_artifact, build_network_artifact_with_progress, run_search_request,
    run_search_request_with_progress,
};

/// In-memory state shared across HTTP connections.
#[derive(Default)]
struct ServerState {
    /// Monotonic counter used to allocate new job identifiers.
    next_job_id: AtomicU64,
    /// Active and completed jobs keyed by server-assigned identifier.
    jobs: Mutex<HashMap<u64, StoredJob>>,
}

/// Current lifecycle state for an asynchronous job.
#[derive(Clone)]
enum StoredJob {
    /// Job has been accepted but worker execution has not started yet.
    Queued(Arc<JobProgressTracker>),
    /// Job is actively computing and reporting progress updates.
    Running(Arc<JobProgressTracker>),
    /// Job completed successfully and the result payload is available.
    Finished(MatcherJobResult),
    /// Job terminated with an error, optionally preserving the last progress snapshot.
    Failed {
        error: String,
        progress: Option<JobProgress>,
    },
}

/// Thread-safe progress tracker shared between the worker and polling endpoints.
struct JobProgressTracker {
    /// Current coarse-grained stage in the pipeline.
    stage: Mutex<JobProgressStage>,
    /// Completed work units for the active stage.
    completed: AtomicU64,
    /// Total work units expected for the active stage.
    total: AtomicU64,
    /// Cooperative cancellation flag set by the cancel endpoint.
    cancelled: AtomicBool,
}

impl Default for JobProgressTracker {
    fn default() -> Self {
        Self {
            stage: Mutex::new(JobProgressStage::Queued),
            completed: AtomicU64::new(0),
            total: AtomicU64::new(1),
            cancelled: AtomicBool::new(false),
        }
    }
}

impl JobProgressTracker {
    /// Replaces the current progress snapshot with a new stage and counters.
    fn set(&self, stage: JobProgressStage, completed: u64, total: u64) {
        if let Ok(mut slot) = self.stage.lock() {
            *slot = stage;
        }
        self.completed.store(completed, Ordering::Relaxed);
        self.total.store(total.max(1), Ordering::Relaxed);
    }

    /// Captures the latest externally visible progress state.
    fn snapshot(&self) -> JobProgress {
        let stage = self
            .stage
            .lock()
            .map(|slot| *slot)
            .unwrap_or(JobProgressStage::Queued);
        JobProgress {
            stage,
            completed: self.completed.load(Ordering::Relaxed),
            total: self.total.load(Ordering::Relaxed).max(1),
        }
    }

    /// Requests cooperative cancellation for the associated worker.
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    /// Returns whether cancellation has been requested.
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

/// Starts the HTTP server on the provided `host:port` binding.
pub fn serve(bind: &str) -> Result<(), String> {
    let listener =
        TcpListener::bind(bind).map_err(|err| format!("failed to bind {bind}: {err}"))?;
    let state = Arc::new(ServerState::default());

    for stream in listener.incoming() {
        let stream = match stream {
            Ok(stream) => stream,
            Err(err) => {
                eprintln!("accept failed: {err}");
                continue;
            }
        };
        let state = Arc::clone(&state);
        std::thread::spawn(move || {
            if let Err(err) = handle_connection(stream, &state) {
                eprintln!("connection failed: {err}");
            }
        });
    }

    Ok(())
}

/// Handles a single HTTP connection from request parsing through response writeback.
fn handle_connection(mut stream: TcpStream, state: &Arc<ServerState>) -> Result<(), String> {
    let request = read_request(&mut stream)?;
    let response = handle_request(request, state);
    stream
        .write_all(&response)
        .map_err(|err| format!("failed to write response: {err}"))?;
    stream
        .flush()
        .map_err(|err| format!("failed to flush response: {err}"))
}

/// Minimal request representation used by the server router.
struct HttpRequest {
    /// Uppercase HTTP method such as `GET` or `POST`.
    method: String,
    /// Request target path without scheme or authority.
    path: String,
    /// Raw request body bytes.
    body: Vec<u8>,
}

/// Reads an HTTP/1.1 request from a TCP stream into an [`HttpRequest`].
fn read_request(stream: &mut TcpStream) -> Result<HttpRequest, String> {
    let mut buffer = Vec::new();
    let mut header_end = None;
    while header_end.is_none() {
        let mut chunk = [0_u8; 4096];
        let read = stream
            .read(&mut chunk)
            .map_err(|err| format!("failed to read request: {err}"))?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        header_end = find_header_end(&buffer);
    }
    let header_end = header_end.ok_or("malformed HTTP request: missing header terminator")?;
    let header_bytes = &buffer[..header_end];
    let header_text = String::from_utf8(header_bytes.to_vec())
        .map_err(|_| "request headers are not valid UTF-8".to_string())?;
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().ok_or("missing HTTP request line")?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or("missing HTTP method")?.to_string();
    let path = parts.next().ok_or("missing HTTP path")?.to_string();
    let mut content_length = 0usize;
    for line in lines {
        if let Some((name, value)) = line.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            content_length = value
                .trim()
                .parse::<usize>()
                .map_err(|_| "invalid Content-Length header".to_string())?;
        }
    }

    let mut body = buffer[header_end + 4..].to_vec();
    if body.len() < content_length {
        let mut remaining = vec![0_u8; content_length - body.len()];
        stream
            .read_exact(&mut remaining)
            .map_err(|err| format!("failed to read request body: {err}"))?;
        body.extend_from_slice(&remaining);
    } else if body.len() > content_length {
        body.truncate(content_length);
    }

    Ok(HttpRequest { method, path, body })
}

/// Finds the byte offset where the HTTP headers end.
fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

/// Routes an HTTP request to the matching endpoint handler.
fn handle_request(request: HttpRequest, state: &Arc<ServerState>) -> Vec<u8> {
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/v1/health") => json_response(200, &HealthResponse { status: "ok" }),
        ("POST", "/v1/network") => {
            match decode_json::<NetworkRequest>(&request.body).and_then(build_network_artifact) {
                Ok(artifact) => json_response(200, &artifact),
                Err(err) => error_response(400, &err),
            }
        }
        ("POST", "/v1/library-search") => {
            match decode_json::<SearchRequest>(&request.body).and_then(run_search_request) {
                Ok(artifact) => json_response(200, &artifact),
                Err(err) => error_response(400, &err),
            }
        }
        ("POST", "/v1/network/jobs") => match decode_json::<NetworkRequest>(&request.body) {
            Ok(payload) => start_job(state, move |progress| {
                let progress_for_updates = Arc::clone(&progress);
                let progress_for_cancel = Arc::clone(&progress);
                build_network_artifact_with_progress(
                    payload,
                    move |stage, completed, total| {
                        progress_for_updates.set(stage, completed, total);
                    },
                    move || progress_for_cancel.is_cancelled(),
                )
                .map(MatcherJobResult::Network)
            }),
            Err(err) => error_response(400, &err),
        },
        ("POST", "/v1/library-search/jobs") => match decode_json::<SearchRequest>(&request.body) {
            Ok(payload) => start_job(state, move |progress| {
                let progress_for_updates = Arc::clone(&progress);
                let progress_for_cancel = Arc::clone(&progress);
                run_search_request_with_progress(
                    payload,
                    move |stage, completed, total| {
                        progress_for_updates.set(stage, completed, total);
                    },
                    move || progress_for_cancel.is_cancelled(),
                )
                .map(MatcherJobResult::LibrarySearch)
            }),
            Err(err) => error_response(400, &err),
        },
        _ => {
            if request.method == "GET" {
                if let Some(job_id) = request.path.strip_prefix("/v1/jobs/") {
                    if let Some(id) = job_id.strip_suffix("/result") {
                        return get_job_result_response(state, id);
                    }
                    return get_job_status_response(state, job_id);
                }
            }
            if request.method == "POST" {
                if let Some(job_id) = request.path.strip_prefix("/v1/jobs/")
                    && let Some(id) = job_id.strip_suffix("/cancel")
                {
                    return cancel_job_response(state, id);
                }
            }
            error_response(404, "not found")
        }
    }
}

/// Creates an asynchronous job entry and starts worker execution in a background thread.
fn start_job<F>(state: &Arc<ServerState>, job_fn: F) -> Vec<u8>
where
    F: FnOnce(Arc<JobProgressTracker>) -> Result<MatcherJobResult, String> + Send + 'static,
{
    let job_id = state.next_job_id.fetch_add(1, Ordering::Relaxed) + 1;
    let progress = Arc::new(JobProgressTracker::default());
    if let Ok(mut jobs) = state.jobs.lock() {
        jobs.insert(job_id, StoredJob::Queued(Arc::clone(&progress)));
    }

    let state = Arc::clone(state);
    std::thread::spawn(move || {
        update_job(&state, job_id, StoredJob::Running(Arc::clone(&progress)));
        match job_fn(Arc::clone(&progress)) {
            Ok(result) => {
                progress.set(JobProgressStage::Finalizing, 1, 1);
                update_job(&state, job_id, StoredJob::Finished(result));
            }
            Err(err) if err == "job cancelled" => update_job(
                &state,
                job_id,
                StoredJob::Failed {
                    error: err,
                    progress: Some(progress.snapshot()),
                },
            ),
            Err(err) => update_job(
                &state,
                job_id,
                StoredJob::Failed {
                    error: err,
                    progress: Some(progress.snapshot()),
                },
            ),
        }
    });

    json_response(
        202,
        &JobCreatedResponse {
            job_id,
            status: JobStatus::Queued,
        },
    )
}

/// Replaces the stored state for a given job identifier.
fn update_job(state: &Arc<ServerState>, job_id: u64, job: StoredJob) {
    if let Ok(mut jobs) = state.jobs.lock() {
        jobs.insert(job_id, job);
    }
}

/// Returns the current status payload for an asynchronous job.
fn get_job_status_response(state: &Arc<ServerState>, raw_job_id: &str) -> Vec<u8> {
    let job_id = match raw_job_id.parse::<u64>() {
        Ok(job_id) => job_id,
        Err(_) => return error_response(400, "invalid job id"),
    };
    let response = match state.jobs.lock() {
        Ok(jobs) => match jobs.get(&job_id) {
            Some(StoredJob::Queued(progress)) => JobStatusResponse {
                job_id,
                status: JobStatus::Queued,
                error: None,
                progress: Some(progress.snapshot()),
            },
            Some(StoredJob::Running(progress)) => JobStatusResponse {
                job_id,
                status: JobStatus::Running,
                error: None,
                progress: Some(progress.snapshot()),
            },
            Some(StoredJob::Finished(_)) => JobStatusResponse {
                job_id,
                status: JobStatus::Finished,
                error: None,
                progress: Some(JobProgress {
                    stage: JobProgressStage::Finalizing,
                    completed: 1,
                    total: 1,
                }),
            },
            Some(StoredJob::Failed { error, progress }) => JobStatusResponse {
                job_id,
                status: JobStatus::Failed,
                error: Some(error.clone()),
                progress: progress.clone(),
            },
            None => return error_response(404, "job not found"),
        },
        Err(_) => return error_response(500, "job store unavailable"),
    };
    json_response(200, &response)
}

/// Marks a queued or running job as cancelled.
fn cancel_job_response(state: &Arc<ServerState>, raw_job_id: &str) -> Vec<u8> {
    let job_id = match raw_job_id.parse::<u64>() {
        Ok(job_id) => job_id,
        Err(_) => return error_response(400, "invalid job id"),
    };

    match state.jobs.lock() {
        Ok(jobs) => match jobs.get(&job_id) {
            Some(StoredJob::Queued(progress)) | Some(StoredJob::Running(progress)) => {
                progress.cancel();
                json_response(
                    200,
                    &JobStatusResponse {
                        job_id,
                        status: JobStatus::Running,
                        error: Some("cancellation requested".to_string()),
                        progress: Some(progress.snapshot()),
                    },
                )
            }
            Some(StoredJob::Finished(_)) => error_response(409, "job is already finished"),
            Some(StoredJob::Failed { .. }) => error_response(409, "job is already finished"),
            None => error_response(404, "job not found"),
        },
        Err(_) => error_response(500, "job store unavailable"),
    }
}

/// Returns the final result payload for a completed asynchronous job.
fn get_job_result_response(state: &Arc<ServerState>, raw_job_id: &str) -> Vec<u8> {
    let job_id = match raw_job_id.parse::<u64>() {
        Ok(job_id) => job_id,
        Err(_) => return error_response(400, "invalid job id"),
    };
    match state.jobs.lock() {
        Ok(jobs) => match jobs.get(&job_id) {
            Some(StoredJob::Finished(result)) => json_response(200, result),
            Some(StoredJob::Failed { error, .. }) => error_response(500, error),
            Some(StoredJob::Queued(_)) | Some(StoredJob::Running(_)) => {
                error_response(409, "job is not finished yet")
            }
            None => error_response(404, "job not found"),
        },
        Err(_) => error_response(500, "job store unavailable"),
    }
}

/// Deserializes a JSON request body into the target type.
fn decode_json<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, String> {
    serde_json::from_slice(body).map_err(|err| format!("invalid JSON request: {err}"))
}

/// Serializes a value into a JSON HTTP response.
fn json_response<T: serde::Serialize>(status: u16, value: &T) -> Vec<u8> {
    match serde_json::to_vec(value) {
        Ok(body) => raw_response(status, "application/json", &body),
        Err(err) => error_response(500, &format!("failed to serialize response: {err}")),
    }
}

/// Builds a JSON error response containing a single `error` message field.
fn error_response(status: u16, message: &str) -> Vec<u8> {
    let body = format!(r#"{{"error":"{}"}}"#, escape_json_string(message));
    raw_response(status, "application/json", body.as_bytes())
}

/// Builds a complete HTTP/1.1 response with standard headers.
fn raw_response(status: u16, content_type: &str, body: &[u8]) -> Vec<u8> {
    let status_text = match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        404 => "Not Found",
        409 => "Conflict",
        500 => "Internal Server Error",
        _ => "OK",
    };
    let mut response = format!(
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
        body.len()
    )
    .into_bytes();
    response.extend_from_slice(body);
    response
}

/// Escapes a plain string for safe inclusion in a minimal JSON string literal.
fn escape_json_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

#[cfg(test)]
mod tests {
    use super::raw_response;

    /// Ensures raw responses advertise the actual payload size.
    #[test]
    fn raw_response_contains_content_length() {
        let response =
            String::from_utf8(raw_response(200, "application/json", br#"{"ok":1}"#)).expect("utf8");
        assert!(response.contains("Content-Length: 8"));
    }
}
