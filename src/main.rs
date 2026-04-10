//! Native CLI entrypoint for matcher search, network, metric, and database utilities.

#[cfg(not(target_arch = "wasm32"))]
use std::env;
#[cfg(not(target_arch = "wasm32"))]
use std::io::{self, IsTerminal, Write};
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Mutex;
#[cfg(not(target_arch = "wasm32"))]
use std::time::{Duration, Instant};

#[cfg(not(target_arch = "wasm32"))]
use serde::Deserialize;

#[cfg(not(target_arch = "wasm32"))]
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};

#[cfg(not(target_arch = "wasm32"))]
use spectral_matcher::{
    ConsensusMergeParams, JobProgressStage, NetworkBuildParams, ParseConfig, SearchArtifact,
    SearchQueryKey, SearchRequest, SimilarityMetric, build_network_artifact_with_progress,
    download_spectral_database, merge_search_artifacts, resolve_spectral_database,
    run_search_request_with_progress, save_json_to_path, save_tsv_to_path, serve,
    spectral_databases,
};

/// Batch config for one or more library-search jobs.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Deserialize)]
struct SearchBatchConfig {
    /// Optional base directory used to derive per-job output paths from the job name.
    #[serde(default)]
    output_dir: Option<PathBuf>,
    /// Search jobs executed from this batch file.
    jobs: Vec<SearchJobConfig>,
}

/// One CLI-configured library-search job.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Deserialize)]
struct SearchJobConfig {
    /// Human-readable job name used in logs and, when `output_dir` is set, output paths.
    name: Option<String>,
    /// Query MGF input path.
    query_mgf: PathBuf,
    /// Library MGF input path.
    library_mgf: PathBuf,
    /// Optional explicit JSON output path overriding derived batch output locations.
    #[serde(default)]
    output_json: PathBuf,
    /// Optional explicit TSV output path overriding derived batch output locations.
    #[serde(default)]
    output_tsv: Option<PathBuf>,
    #[serde(default)]
    parse: ParseConfig,
    search: SearchConfig,
    #[serde(default)]
    taxonomy: Option<SearchTaxonomyConfig>,
    #[serde(default)]
    output: SearchOutputConfig,
}

/// Search-scoring configuration accepted from a CLI TOML file.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Deserialize)]
struct SearchConfig {
    metric: spectral_matcher::SimilarityMetric,
    fragment_mz_tolerance: f64,
    mz_power: f64,
    intensity_power: f64,
    #[serde(default)]
    top_n_peaks: Option<usize>,
    precursor_mz_tolerance: f64,
    min_matched_peaks: usize,
    min_similarity_threshold: f64,
    top_n: usize,
}

/// Output-export options for a CLI search job.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Deserialize, Default)]
struct SearchOutputConfig {
    query_key: Option<SearchQueryKey>,
}

/// Taxonomy-reranking configuration accepted from a CLI TOML file.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Deserialize)]
struct SearchTaxonomyConfig {
    query: String,
    lotus_csv: PathBuf,
}

/// Batch config for one or more network-build jobs.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Deserialize)]
struct NetworkBatchConfig {
    /// Optional base directory used to derive per-job output paths from the job name.
    #[serde(default)]
    output_dir: Option<PathBuf>,
    /// Network jobs executed from this batch file.
    jobs: Vec<NetworkJobConfig>,
}

/// One CLI-configured network-build job.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Deserialize)]
struct NetworkJobConfig {
    /// Human-readable job name used in logs and, when `output_dir` is set, output paths.
    name: Option<String>,
    /// Input MGF used to construct the spectral network.
    input_mgf: PathBuf,
    /// Optional explicit JSON output path overriding derived batch output locations.
    #[serde(default)]
    output_json: PathBuf,
    /// Optional explicit CSV directory overriding derived batch output locations.
    #[serde(default)]
    output_csv_dir: Option<PathBuf>,
    #[serde(default)]
    parse: ParseConfig,
    build: NetworkBuildParams,
}

/// Batch config for one or more cross-library consensus jobs.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Deserialize)]
struct ConsensusBatchConfig {
    /// Optional base directory used to derive per-job output paths from the job name.
    #[serde(default)]
    output_dir: Option<PathBuf>,
    /// Consensus jobs executed from this batch file.
    jobs: Vec<ConsensusJobConfig>,
}

/// One CLI-configured cross-library consensus job.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Deserialize)]
struct ConsensusJobConfig {
    /// Human-readable job name used in logs and, when `output_dir` is set, output paths.
    name: Option<String>,
    /// First input search artifact to merge.
    left_search_json: PathBuf,
    /// Second input search artifact to merge.
    right_search_json: PathBuf,
    /// Optional explicit alias for the first input artifact.
    #[serde(default)]
    left_name: Option<String>,
    /// Optional explicit alias for the second input artifact.
    #[serde(default)]
    right_name: Option<String>,
    /// Optional explicit JSON output path overriding derived batch output locations.
    #[serde(default)]
    output_json: PathBuf,
    /// Optional explicit TSV output path overriding derived batch output locations.
    #[serde(default)]
    output_tsv: Option<PathBuf>,
    /// Consensus-fusion parameters.
    #[serde(default)]
    merge: ConsensusMergeParams,
    /// Output-export options for the merged artifact.
    #[serde(default)]
    output: ConsensusOutputConfig,
}

/// Output-export options for a CLI consensus job.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Deserialize, Default)]
struct ConsensusOutputConfig {
    /// Optional query identifier mode overriding the query key stored in the input artifacts.
    query_key: Option<SearchQueryKey>,
}

/// Mutable render state for the lightweight CLI progress display.
#[cfg(not(target_arch = "wasm32"))]
struct CliProgressState {
    last_stage: Option<JobProgressStage>,
    last_percent: u64,
    last_render: Option<Instant>,
    last_width: usize,
    rendered: bool,
}

/// Terminal-oriented progress renderer used by `search` and `network` commands.
#[cfg(not(target_arch = "wasm32"))]
struct CliProgress {
    prefix: String,
    enabled: bool,
    state: Mutex<CliProgressState>,
}

/// Resolved output paths for a search job.
#[cfg(not(target_arch = "wasm32"))]
struct ResolvedSearchOutputs {
    /// Final JSON artifact path.
    json: PathBuf,
    /// Optional TSV export path.
    tsv: Option<PathBuf>,
}

/// Resolved output paths for a network job.
#[cfg(not(target_arch = "wasm32"))]
struct ResolvedNetworkOutputs {
    /// Final JSON artifact path.
    json: PathBuf,
    /// Optional CSV companion directory.
    csv_dir: Option<PathBuf>,
}

/// Resolved output paths for a consensus job.
#[cfg(not(target_arch = "wasm32"))]
struct ResolvedConsensusOutputs {
    /// Final JSON artifact path.
    json: PathBuf,
    /// Optional TSV export path.
    tsv: Option<PathBuf>,
}

#[cfg(not(target_arch = "wasm32"))]
impl CliProgress {
    /// Creates a progress renderer with a fixed prefix such as a job label.
    fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            enabled: io::stderr().is_terminal(),
            state: Mutex::new(CliProgressState {
                last_stage: None,
                last_percent: 0,
                last_render: None,
                last_width: 0,
                rendered: false,
            }),
        }
    }

    /// Updates the progress display for the current stage.
    fn update(&self, stage: JobProgressStage, completed: u64, total: u64) {
        if !self.enabled {
            return;
        }

        let total = total.max(1);
        let completed = completed.min(total);
        let percent = completed.saturating_mul(100) / total;
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let now = Instant::now();
        let stage_changed = state.last_stage != Some(stage);
        let percent_changed = state.last_percent != percent;
        let is_complete = completed >= total;
        let due = state
            .last_render
            .is_none_or(|last| now.duration_since(last) >= Duration::from_millis(100));

        if !stage_changed && !is_complete && (!percent_changed || !due) {
            return;
        }

        let message = format!(
            "\r{} {} {} {:>3}% {}/{}",
            self.prefix,
            render_bar(percent),
            progress_stage_label(stage),
            percent,
            completed,
            total
        );
        let pad = state.last_width.saturating_sub(message.len());
        eprint!("{message}{:pad$}", "");
        let _ = io::stderr().flush();

        state.last_stage = Some(stage);
        state.last_percent = percent;
        state.last_render = Some(now);
        state.last_width = message.len();
        state.rendered = true;
    }

    /// Finalizes the display with a one-line summary.
    fn finish(&self, summary: &str) {
        if !self.enabled {
            return;
        }
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if !state.rendered {
            return;
        }
        let message = format!("\r{} {summary}", self.prefix);
        let pad = state.last_width.saturating_sub(message.len());
        eprintln!("{message}{:pad$}", "");
        state.last_width = 0;
        state.rendered = false;
    }
}

/// Renders a compact ASCII progress bar for CLI stage updates.
#[cfg(not(target_arch = "wasm32"))]
fn render_bar(percent: u64) -> String {
    let width = 24usize;
    let filled = ((percent.min(100) as usize) * width) / 100;
    format!(
        "[{}{}]",
        "#".repeat(filled),
        "-".repeat(width.saturating_sub(filled))
    )
}

/// Maps internal progress stages to user-facing CLI labels.
#[cfg(not(target_arch = "wasm32"))]
fn progress_stage_label(stage: JobProgressStage) -> &'static str {
    match stage {
        JobProgressStage::Queued => "queued",
        JobProgressStage::LoadingSpectra => "loading spectra",
        JobProgressStage::LoadingQuery => "loading query",
        JobProgressStage::LoadingLibrary => "loading library",
        JobProgressStage::LoadingTaxonomy => "loading taxonomy",
        JobProgressStage::Scoring => "scoring",
        JobProgressStage::TaxonomicReranking => "taxonomic reranking",
        JobProgressStage::BuildingNetwork => "building network",
        JobProgressStage::Finalizing => "finalizing",
    }
}

/// CLI entrypoint for native and wasm builds.
fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Err(err) = run(env::args()) {
            eprintln!("{err}");
            std::process::exit(1);
        }
    }

    #[cfg(target_arch = "wasm32")]
    {
        eprintln!("spectral-matcher CLI is unavailable on wasm");
        std::process::exit(1);
    }
}

/// Dispatches CLI subcommands.
#[cfg(not(target_arch = "wasm32"))]
fn run<I>(mut args: I) -> Result<(), String>
where
    I: Iterator<Item = String>,
{
    let _exe = args.next();
    let Some(command) = args.next() else {
        return Err(
            "usage: spectral-matcher <serve|search|network|consensus|metrics|db> ..."
                .to_string(),
        );
    };
    match command.as_str() {
        "serve" => {
            let bind = parse_bind_arg(args)?;
            serve(&bind)
        }
        "search" => {
            let path = parse_config_arg(args)?;
            run_search_config(Path::new(&path))
        }
        "network" => {
            let path = parse_config_arg(args)?;
            run_network_config(Path::new(&path))
        }
        "consensus" => {
            let path = parse_config_arg(args)?;
            run_consensus_config(Path::new(&path))
        }
        "metrics" => {
            if args.next().is_some() {
                return Err("unexpected extra arguments".to_string());
            }
            print_metrics();
            Ok(())
        }
        "db" => run_database_command(args),
        other => Err(format!("unsupported command '{other}'")),
    }
}

/// Dispatches `db` subcommands for listing and downloading curated spectral databases.
#[cfg(not(target_arch = "wasm32"))]
fn run_database_command<I>(mut args: I) -> Result<(), String>
where
    I: Iterator<Item = String>,
{
    let Some(command) = args.next() else {
        return Err("usage: spectral-matcher db <list|download> ...".to_string());
    };
    match command.as_str() {
        "list" => {
            if args.next().is_some() {
                return Err("unexpected extra arguments".to_string());
            }
            print_database_list();
            Ok(())
        }
        "download" => {
            let Some(selection) = args.next() else {
                return Err(
                    "usage: spectral-matcher db download <database-id|all> [--output-dir <path>]"
                        .to_string(),
                );
            };
            let mut output_dir = PathBuf::from("databases");
            while let Some(arg) = args.next() {
                if arg == "--output-dir" {
                    let Some(path) = args.next() else {
                        return Err("missing path after --output-dir".to_string());
                    };
                    output_dir = PathBuf::from(path);
                } else {
                    return Err(format!("unsupported argument '{arg}'"));
                }
            }
            download_database_selection(&selection, &output_dir)
        }
        other => Err(format!("unsupported db command '{other}'")),
    }
}

/// Prints the curated spectral-database registry.
#[cfg(not(target_arch = "wasm32"))]
fn print_database_list() {
    println!("Available spectral databases:");
    for database in spectral_databases() {
        println!(
            "- {}: {} | {} | {}",
            database.id, database.name, database.category, database.dimensions
        );
        println!("  filename: {}", database.filename);
        println!("  description: {}", database.description);
        println!("  url: {}", database.url);
    }
}

/// Downloads one or more curated databases selected by id or by the special `all` token.
#[cfg(not(target_arch = "wasm32"))]
fn download_database_selection(selection: &str, output_dir: &Path) -> Result<(), String> {
    let databases = if selection.eq_ignore_ascii_case("all") {
        spectral_databases().to_vec()
    } else {
        vec![*resolve_spectral_database(selection).ok_or_else(|| {
            format!(
                "unknown database '{}'; run `spectral-matcher db list` to see valid ids",
                selection
            )
        })?]
    };

    for database in databases {
        let interactive = io::stderr().is_terminal();
        let pb = build_download_progress_bar(&database.name);
        let mut known_total = None;
        let path = download_spectral_database(&database, output_dir, |downloaded, total| {
            if total != known_total {
                known_total = total;
                if let Some(total) = total {
                    pb.set_length(total);
                    pb.set_style(download_bar_style());
                } else {
                    pb.set_style(download_spinner_style());
                }
            }
            pb.set_position(downloaded);
            if total.is_none() {
                pb.set_message(format!("{} {}", database.name, human_bytes(downloaded)));
            }
        })?;
        pb.finish_with_message(format!(
            "{} downloaded to {}",
            database.name,
            path.display()
        ));
        if !interactive {
            println!("{} -> {}", database.id, path.display());
        }
    }
    Ok(())
}

/// Creates a progress bar for a single database download.
#[cfg(not(target_arch = "wasm32"))]
fn build_download_progress_bar(name: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_draw_target(if io::stderr().is_terminal() {
        ProgressDrawTarget::stderr()
    } else {
        ProgressDrawTarget::hidden()
    });
    pb.enable_steady_tick(Duration::from_millis(100));
    pb.set_style(download_spinner_style());
    pb.set_message(name.to_string());
    pb
}

/// Spinner style used before a download has a known content length.
#[cfg(not(target_arch = "wasm32"))]
fn download_spinner_style() -> ProgressStyle {
    ProgressStyle::with_template("{spinner:.green} {msg}")
        .expect("spinner template")
        .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ")
}

/// Full progress-bar style used when the remote server reports content length.
#[cfg(not(target_arch = "wasm32"))]
fn download_bar_style() -> ProgressStyle {
    ProgressStyle::with_template(
        "{spinner:.green} {msg} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, eta {eta})",
    )
    .expect("bar template")
    .progress_chars("=> ")
}

/// Formats a byte count using human-readable binary units.
#[cfg(not(target_arch = "wasm32"))]
fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Prints the currently supported similarity metrics and their descriptions.
#[cfg(not(target_arch = "wasm32"))]
fn print_metrics() {
    println!("Available similarity metrics:");
    for metric in SimilarityMetric::ALL {
        let suffix = if metric == SimilarityMetric::default() {
            " (default)"
        } else {
            ""
        };
        println!("- {}{}: {}", metric.label(), suffix, metric.description());
    }
    println!();
    println!("Use one of these values in `[jobs.search].metric` or `[jobs.build.compute].metric`.");
}

/// Parses the optional `--bind` argument for the HTTP server.
#[cfg(not(target_arch = "wasm32"))]
fn parse_bind_arg<I>(mut args: I) -> Result<String, String>
where
    I: Iterator<Item = String>,
{
    let Some(flag) = args.next() else {
        return Ok("127.0.0.1:8787".to_string());
    };
    if flag != "--bind" {
        return Err(format!("unsupported argument '{flag}', expected --bind"));
    }
    let Some(bind) = args.next() else {
        return Err("missing bind address after --bind".to_string());
    };
    if args.next().is_some() {
        return Err("unexpected extra arguments".to_string());
    }
    Ok(bind)
}

/// Parses the required `--config <path>` argument for config-driven commands.
#[cfg(not(target_arch = "wasm32"))]
fn parse_config_arg<I>(mut args: I) -> Result<String, String>
where
    I: Iterator<Item = String>,
{
    let Some(flag) = args.next() else {
        return Err("missing --config <path>".to_string());
    };
    if flag != "--config" {
        return Err(format!("unsupported argument '{flag}', expected --config"));
    }
    let Some(path) = args.next() else {
        return Err("missing config path after --config".to_string());
    };
    if args.next().is_some() {
        return Err("unexpected extra arguments".to_string());
    }
    Ok(path)
}

/// Loads and runs every search job declared in a batch config file.
#[cfg(not(target_arch = "wasm32"))]
fn run_search_config(path: &Path) -> Result<(), String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let config: SearchBatchConfig =
        toml::from_str(&raw).map_err(|err| format!("failed to parse {}: {err}", path.display()))?;
    if config.jobs.is_empty() {
        return Err("config must contain at least one [[jobs]] entry".to_string());
    }
    let output_dir = config.output_dir;
    for (idx, job) in config.jobs.into_iter().enumerate() {
        let label = job
            .name
            .clone()
            .unwrap_or_else(|| format!("job {}", idx + 1));
        run_search_job(&label, output_dir.as_deref(), job)?;
    }
    Ok(())
}

/// Executes a single search job from a CLI config file.
#[cfg(not(target_arch = "wasm32"))]
fn run_search_job(label: &str, batch_output_dir: Option<&Path>, job: SearchJobConfig) -> Result<(), String> {
    let outputs = resolve_search_outputs(batch_output_dir, label, &job)?;
    let request = SearchRequest {
        query_source_label: job.query_mgf.display().to_string(),
        query_mgf_text: None,
        query_mgf_path: Some(job.query_mgf.display().to_string()),
        library_source_label: job.library_mgf.display().to_string(),
        library_mgf_text: None,
        library_mgf_path: Some(job.library_mgf.display().to_string()),
        parse: job.parse,
        search: spectral_matcher::LibrarySearchParams {
            compute: spectral_matcher::ComputeParams {
                metric: job.search.metric,
                fragment_mz_tolerance: job.search.fragment_mz_tolerance,
                mz_power: job.search.mz_power,
                intensity_power: job.search.intensity_power,
                top_n_peaks: job.search.top_n_peaks,
            },
            precursor_mz_tolerance: job.search.precursor_mz_tolerance,
            min_matched_peaks: job.search.min_matched_peaks,
            min_similarity_threshold: job.search.min_similarity_threshold,
            top_n: job.search.top_n,
        },
        taxonomy: job.taxonomy.map(|taxonomy| spectral_matcher::SearchTaxonomyRequest {
            query_text: taxonomy.query,
            lotus_source_label: taxonomy.lotus_csv.display().to_string(),
            lotus_csv_text: None,
            lotus_csv_path: Some(taxonomy.lotus_csv.display().to_string()),
        }),
        query_key: job.output.query_key,
    };
    let progress = CliProgress::new(format!("[search:{label}]"));
    let artifact = run_search_request_with_progress(
        request,
        |stage, completed, total| progress.update(stage, completed, total),
        || false,
    )
    .map_err(|err| {
        progress.finish("failed");
        format!("{label}: search failed: {err}")
    })?;
    progress.finish("done");
    let json = serde_json::to_string_pretty(&artifact)
        .map_err(|err| format!("{label}: failed to serialize JSON output: {err}"))?;
    save_json_to_path(&outputs.json, &json)
        .map_err(|err| format!("{label}: failed to write JSON output: {err}"))?;
    if let Some(path) = outputs.tsv {
        save_tsv_to_path(&path, &artifact.tsv)
            .map_err(|err| format!("{label}: failed to write TSV output: {err}"))?;
    }
    Ok(())
}

/// Loads and runs every network job declared in a batch config file.
#[cfg(not(target_arch = "wasm32"))]
fn run_network_config(path: &Path) -> Result<(), String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let config: NetworkBatchConfig =
        toml::from_str(&raw).map_err(|err| format!("failed to parse {}: {err}", path.display()))?;
    if config.jobs.is_empty() {
        return Err("config must contain at least one [[jobs]] entry".to_string());
    }
    let output_dir = config.output_dir;
    for (idx, job) in config.jobs.into_iter().enumerate() {
        let label = job
            .name
            .clone()
            .unwrap_or_else(|| format!("job {}", idx + 1));
        run_network_job(&label, output_dir.as_deref(), job)?;
    }
    Ok(())
}

/// Executes a single network-build job from a CLI config file.
#[cfg(not(target_arch = "wasm32"))]
fn run_network_job(
    label: &str,
    batch_output_dir: Option<&Path>,
    job: NetworkJobConfig,
) -> Result<(), String> {
    let outputs = resolve_network_outputs(batch_output_dir, label, &job)?;
    let request = spectral_matcher::NetworkRequest {
        source_label: job.input_mgf.display().to_string(),
        mgf_text: None,
        mgf_path: Some(job.input_mgf.display().to_string()),
        parse: job.parse,
        build: job.build,
    };
    let progress = CliProgress::new(format!("[network:{label}]"));
    let artifact = build_network_artifact_with_progress(
        request,
        |stage, completed, total| progress.update(stage, completed, total),
        || false,
    )
    .map_err(|err| {
        progress.finish("failed");
        format!("{label}: network build failed: {err}")
    })?;
    progress.finish("done");
    let json = serde_json::to_string_pretty(&artifact)
        .map_err(|err| format!("{label}: failed to serialize network JSON: {err}"))?;
    save_json_to_path(&outputs.json, &json)
        .map_err(|err| format!("{label}: failed to write network JSON: {err}"))?;
    if let Some(dir) = outputs.csv_dir {
        save_network_csvs(&dir, &artifact)?;
    }
    Ok(())
}

/// Loads and runs every consensus job declared in a batch config file.
#[cfg(not(target_arch = "wasm32"))]
fn run_consensus_config(path: &Path) -> Result<(), String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let config: ConsensusBatchConfig =
        toml::from_str(&raw).map_err(|err| format!("failed to parse {}: {err}", path.display()))?;
    if config.jobs.is_empty() {
        return Err("config must contain at least one [[jobs]] entry".to_string());
    }
    let output_dir = config.output_dir;
    for (idx, job) in config.jobs.into_iter().enumerate() {
        let label = job
            .name
            .clone()
            .unwrap_or_else(|| format!("job {}", idx + 1));
        run_consensus_job(&label, output_dir.as_deref(), job)?;
    }
    Ok(())
}

/// Executes a single consensus job from a CLI config file.
#[cfg(not(target_arch = "wasm32"))]
fn run_consensus_job(
    label: &str,
    batch_output_dir: Option<&Path>,
    job: ConsensusJobConfig,
) -> Result<(), String> {
    let outputs = resolve_consensus_outputs(batch_output_dir, label, &job)?;
    let left = load_search_artifact(&job.left_search_json)
        .map_err(|err| format!("{label}: failed to load left search artifact: {err}"))?;
    let right = load_search_artifact(&job.right_search_json)
        .map_err(|err| format!("{label}: failed to load right search artifact: {err}"))?;
    let left_name = resolve_consensus_input_name(
        job.left_name.as_deref(),
        &left.library_source_label,
        "left",
    );
    let right_name = resolve_consensus_input_name(
        job.right_name.as_deref(),
        &right.library_source_label,
        "right",
    );
    let (left_name, right_name) = ensure_distinct_input_names(left_name, right_name);
    let artifact = merge_search_artifacts(
        &left_name,
        left,
        &right_name,
        right,
        job.merge,
        job.output.query_key,
    )
    .map_err(|err| format!("{label}: consensus merge failed: {err}"))?;
    let json = serde_json::to_string_pretty(&artifact)
        .map_err(|err| format!("{label}: failed to serialize consensus JSON: {err}"))?;
    save_json_to_path(&outputs.json, &json)
        .map_err(|err| format!("{label}: failed to write consensus JSON: {err}"))?;
    if let Some(path) = outputs.tsv {
        save_tsv_to_path(&path, &artifact.tsv)
            .map_err(|err| format!("{label}: failed to write consensus TSV: {err}"))?;
    }
    Ok(())
}

/// Resolves effective JSON and TSV output paths for a search job.
#[cfg(not(target_arch = "wasm32"))]
fn resolve_search_outputs(
    batch_output_dir: Option<&Path>,
    label: &str,
    job: &SearchJobConfig,
) -> Result<ResolvedSearchOutputs, String> {
    if !job.output_json.as_os_str().is_empty() {
        return Ok(ResolvedSearchOutputs {
            json: job.output_json.clone(),
            tsv: job.output_tsv.clone(),
        });
    }
    let Some(root) = batch_output_dir else {
        return Err(format!(
            "{label}: missing output path; set top-level `output_dir` or explicit `output_json`"
        ));
    };
    let job_name = output_job_name(label, job.name.as_deref())?;
    let job_dir = root.join(job_name);
    Ok(ResolvedSearchOutputs {
        json: job_dir.join("search.json"),
        tsv: Some(job_dir.join("search.tsv")),
    })
}

/// Resolves effective JSON and CSV output paths for a network job.
#[cfg(not(target_arch = "wasm32"))]
fn resolve_network_outputs(
    batch_output_dir: Option<&Path>,
    label: &str,
    job: &NetworkJobConfig,
) -> Result<ResolvedNetworkOutputs, String> {
    if !job.output_json.as_os_str().is_empty() {
        return Ok(ResolvedNetworkOutputs {
            json: job.output_json.clone(),
            csv_dir: job.output_csv_dir.clone(),
        });
    }
    let Some(root) = batch_output_dir else {
        return Err(format!(
            "{label}: missing output path; set top-level `output_dir` or explicit `output_json`"
        ));
    };
    let job_name = output_job_name(label, job.name.as_deref())?;
    let job_dir = root.join(job_name);
    Ok(ResolvedNetworkOutputs {
        json: job_dir.join("network.json"),
        csv_dir: Some(job_dir.join("csv")),
    })
}

/// Validates the job name before it is used as a derived output directory name.
#[cfg(not(target_arch = "wasm32"))]
fn output_job_name<'a>(label: &str, name: Option<&'a str>) -> Result<&'a str, String> {
    let Some(name) = name else {
        return Err(format!(
            "{label}: derived outputs require `name` when using top-level `output_dir`"
        ));
    };
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(format!(
            "{label}: derived outputs require a non-empty `name`"
        ));
    }
    if trimmed == "." || trimmed == ".." || trimmed.contains('/') || trimmed.contains('\\') {
        return Err(format!(
            "{label}: job `name` cannot contain path separators or reserved path segments"
        ));
    }
    Ok(trimmed)
}

/// Writes the network CSV companion files used by downstream visualization tools.
#[cfg(not(target_arch = "wasm32"))]
fn save_network_csvs(
    dir: &Path,
    artifact: &spectral_matcher::NetworkArtifact,
) -> Result<(), String> {
    std::fs::create_dir_all(dir)
        .map_err(|err| format!("failed to create {}: {err}", dir.display()))?;
    let nodes_path = dir.join("nodes.csv");
    let edges_path = dir.join("edges.csv");

    let exported_ids = artifact
        .network
        .nodes
        .iter()
        .map(exported_network_node_id)
        .collect::<Vec<_>>();

    let mut nodes_csv = String::from("node_id,precursor_mz,num_peaks,component_id,degree\n");
    for node in &artifact.network.nodes {
        let node_id = &exported_ids[node.id];
        nodes_csv.push_str(&format!(
            "{},{:.6},{},{},{}\n",
            escape_csv(node_id),
            node.precursor_mz,
            node.num_peaks,
            node.component_id,
            node.degree
        ));
    }
    let mut edges_csv = String::from("source,target,score,matches\n");
    for edge in &artifact.network.edges {
        edges_csv.push_str(&format!(
            "{},{},{:.8},{}\n",
            escape_csv(&exported_ids[edge.source]),
            escape_csv(&exported_ids[edge.target]),
            edge.score,
            edge.matches
        ));
    }
    std::fs::write(&nodes_path, nodes_csv)
        .map_err(|err| format!("failed to write {}: {err}", nodes_path.display()))?;
    std::fs::write(&edges_path, edges_csv)
        .map_err(|err| format!("failed to write {}: {err}", edges_path.display()))?;
    Ok(())
}

/// Resolves effective JSON and TSV output paths for a consensus job.
#[cfg(not(target_arch = "wasm32"))]
fn resolve_consensus_outputs(
    batch_output_dir: Option<&Path>,
    label: &str,
    job: &ConsensusJobConfig,
) -> Result<ResolvedConsensusOutputs, String> {
    if !job.output_json.as_os_str().is_empty() {
        return Ok(ResolvedConsensusOutputs {
            json: job.output_json.clone(),
            tsv: job.output_tsv.clone(),
        });
    }
    let Some(root) = batch_output_dir else {
        return Err(format!(
            "{label}: missing output path; set top-level `output_dir` or explicit `output_json`"
        ));
    };
    let job_name = output_job_name(label, job.name.as_deref())?;
    let job_dir = root.join(job_name);
    Ok(ResolvedConsensusOutputs {
        json: job_dir.join("consensus.json"),
        tsv: Some(job_dir.join("consensus.tsv")),
    })
}

/// Returns the stable exported node id for CSV output.
#[cfg(not(target_arch = "wasm32"))]
fn exported_network_node_id(node: &spectral_matcher::NetworkNode) -> String {
    node.spectrum_id.clone()
}

/// Escapes a CSV cell using minimal quoting.
#[cfg(not(target_arch = "wasm32"))]
fn escape_csv(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') {
        let escaped = value.replace('"', "\"\"");
        format!("\"{escaped}\"")
    } else {
        value.to_string()
    }
}

/// Loads a previously exported search artifact JSON from disk.
#[cfg(not(target_arch = "wasm32"))]
fn load_search_artifact(path: &Path) -> Result<SearchArtifact, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    serde_json::from_str(&raw)
        .map_err(|err| format!("failed to parse {}: {err}", path.display()))
}

/// Resolves the user-facing alias for one input consensus artifact.
#[cfg(not(target_arch = "wasm32"))]
fn resolve_consensus_input_name(explicit: Option<&str>, library_source_label: &str, fallback: &str) -> String {
    let candidate = explicit
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            Path::new(library_source_label)
                .file_stem()
                .and_then(|value| value.to_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| fallback.to_string());
    candidate
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch.to_ascii_lowercase() } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

/// Ensures the two consensus input names remain distinct for export and provenance.
#[cfg(not(target_arch = "wasm32"))]
fn ensure_distinct_input_names(left: String, right: String) -> (String, String) {
    if left != right {
        return (left, right);
    }
    (left, format!("{right}_2"))
}
