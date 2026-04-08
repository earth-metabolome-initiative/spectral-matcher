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
    JobProgressStage, NetworkBuildParams, ParseConfig, SearchQueryKey, SearchRequest,
    SimilarityMetric, download_spectral_database, resolve_spectral_database, spectral_databases,
    build_network_artifact_with_progress, run_search_request_with_progress, save_json_to_path,
    save_tsv_to_path, serve,
};

#[cfg(not(target_arch = "wasm32"))]
#[derive(Deserialize)]
struct SearchBatchConfig {
    jobs: Vec<SearchJobConfig>,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Deserialize)]
struct SearchJobConfig {
    name: Option<String>,
    query_mgf: PathBuf,
    library_mgf: PathBuf,
    output_json: PathBuf,
    output_tsv: Option<PathBuf>,
    #[serde(default)]
    parse: ParseConfig,
    search: SearchConfig,
    #[serde(default)]
    taxonomy: Option<SearchTaxonomyConfig>,
    #[serde(default)]
    output: SearchOutputConfig,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Deserialize)]
struct SearchConfig {
    metric: spectral_matcher::SimilarityMetric,
    fragment_mz_tolerance: f64,
    mz_power: f64,
    intensity_power: f64,
    precursor_mz_tolerance: f64,
    min_matched_peaks: usize,
    min_similarity_threshold: f64,
    top_n: usize,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Deserialize, Default)]
struct SearchOutputConfig {
    query_key: Option<SearchQueryKey>,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Deserialize)]
struct SearchTaxonomyConfig {
    query: String,
    lotus_csv: PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Deserialize)]
struct NetworkBatchConfig {
    jobs: Vec<NetworkJobConfig>,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Deserialize)]
struct NetworkJobConfig {
    name: Option<String>,
    input_mgf: PathBuf,
    output_json: PathBuf,
    output_csv_dir: Option<PathBuf>,
    #[serde(default)]
    parse: ParseConfig,
    build: NetworkBuildParams,
}

#[cfg(not(target_arch = "wasm32"))]
struct CliProgressState {
    last_stage: Option<JobProgressStage>,
    last_percent: u64,
    last_render: Option<Instant>,
    last_width: usize,
    rendered: bool,
}

#[cfg(not(target_arch = "wasm32"))]
struct CliProgress {
    prefix: String,
    enabled: bool,
    state: Mutex<CliProgressState>,
}

#[cfg(not(target_arch = "wasm32"))]
impl CliProgress {
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

#[cfg(not(target_arch = "wasm32"))]
fn run<I>(mut args: I) -> Result<(), String>
where
    I: Iterator<Item = String>,
{
    let _exe = args.next();
    let Some(command) = args.next() else {
        return Err("usage: spectral-matcher <serve|search|network|metrics|db> ...".to_string());
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

#[cfg(not(target_arch = "wasm32"))]
fn download_spinner_style() -> ProgressStyle {
    ProgressStyle::with_template("{spinner:.green} {msg}")
        .expect("spinner template")
        .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ")
}

#[cfg(not(target_arch = "wasm32"))]
fn download_bar_style() -> ProgressStyle {
    ProgressStyle::with_template(
        "{spinner:.green} {msg} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, eta {eta})",
    )
    .expect("bar template")
    .progress_chars("=> ")
}

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

#[cfg(not(target_arch = "wasm32"))]
fn run_search_config(path: &Path) -> Result<(), String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let config: SearchBatchConfig =
        toml::from_str(&raw).map_err(|err| format!("failed to parse {}: {err}", path.display()))?;
    if config.jobs.is_empty() {
        return Err("config must contain at least one [[jobs]] entry".to_string());
    }
    for (idx, job) in config.jobs.into_iter().enumerate() {
        let label = job
            .name
            .clone()
            .unwrap_or_else(|| format!("job {}", idx + 1));
        run_search_job(&label, job)?;
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn run_search_job(label: &str, job: SearchJobConfig) -> Result<(), String> {
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
                top_n_peaks: None,
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
    save_json_to_path(&job.output_json, &json)
        .map_err(|err| format!("{label}: failed to write JSON output: {err}"))?;
    if let Some(path) = job.output_tsv {
        save_tsv_to_path(&path, &artifact.tsv)
            .map_err(|err| format!("{label}: failed to write TSV output: {err}"))?;
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn run_network_config(path: &Path) -> Result<(), String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let config: NetworkBatchConfig =
        toml::from_str(&raw).map_err(|err| format!("failed to parse {}: {err}", path.display()))?;
    if config.jobs.is_empty() {
        return Err("config must contain at least one [[jobs]] entry".to_string());
    }
    for (idx, job) in config.jobs.into_iter().enumerate() {
        let label = job
            .name
            .clone()
            .unwrap_or_else(|| format!("job {}", idx + 1));
        run_network_job(&label, job)?;
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn run_network_job(label: &str, job: NetworkJobConfig) -> Result<(), String> {
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
    save_json_to_path(&job.output_json, &json)
        .map_err(|err| format!("{label}: failed to write network JSON: {err}"))?;
    if let Some(dir) = job.output_csv_dir {
        save_network_csvs(&dir, &artifact)?;
    }
    Ok(())
}

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

#[cfg(not(target_arch = "wasm32"))]
fn exported_network_node_id(node: &spectral_matcher::NetworkNode) -> String {
    node.feature_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| (node.id + 1).to_string())
}

#[cfg(not(target_arch = "wasm32"))]
fn escape_csv(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') {
        let escaped = value.replace('"', "\"\"");
        format!("\"{escaped}\"")
    } else {
        value.to_string()
    }
}
