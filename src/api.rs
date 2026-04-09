//! Serializable request, artifact, and job-status types shared across the matcher interfaces.

use serde::{Deserialize, Serialize};

use crate::export::SearchQueryKey;
use crate::model::{ParseStats, SpectrumMetadata};
use crate::network::SpectralNetwork;
use crate::search::LibrarySearchParams;
use crate::similarity::ComputeParams;

/// Generic parsing limits applied while reading MGF data.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParseConfig {
    /// Minimum number of retained peaks required for a spectrum to be accepted.
    #[serde(default = "default_min_peaks")]
    pub min_peaks: usize,
    /// Maximum number of retained peaks allowed for an accepted spectrum.
    #[serde(default = "default_max_peaks")]
    pub max_peaks: usize,
}

impl Default for ParseConfig {
    fn default() -> Self {
        Self {
            min_peaks: default_min_peaks(),
            max_peaks: default_max_peaks(),
        }
    }
}

/// Parameters that control spectral-network construction once spectra are loaded.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NetworkBuildParams {
    /// Fragment-similarity configuration used to score spectrum pairs.
    pub compute: ComputeParams,
    /// Minimum similarity score required to keep an edge.
    pub threshold: f64,
    /// Maximum number of retained neighbors per node before graph assembly.
    pub top_k: usize,
}

/// Request payload for building a spectral network.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NetworkRequest {
    /// Human-readable label for the MGF source.
    pub source_label: String,
    /// Inline MGF contents, used primarily by HTTP callers and tests.
    #[serde(default)]
    pub mgf_text: Option<String>,
    /// Filesystem path to the MGF file when running natively.
    #[serde(default)]
    pub mgf_path: Option<String>,
    /// Parsing limits for the input spectra.
    #[serde(default)]
    pub parse: ParseConfig,
    /// Network construction parameters.
    pub build: NetworkBuildParams,
}

/// Request payload for searching a query MGF against a library MGF.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchRequest {
    /// Human-readable label for the query source.
    pub query_source_label: String,
    /// Inline query MGF contents.
    #[serde(default)]
    pub query_mgf_text: Option<String>,
    /// Query MGF path for native execution.
    #[serde(default)]
    pub query_mgf_path: Option<String>,
    /// Human-readable label for the library source.
    pub library_source_label: String,
    /// Inline library MGF contents.
    #[serde(default)]
    pub library_mgf_text: Option<String>,
    /// Library MGF path for native execution.
    #[serde(default)]
    pub library_mgf_path: Option<String>,
    /// Parsing limits applied to both query and library spectra.
    #[serde(default)]
    pub parse: ParseConfig,
    /// Spectral-search parameters.
    pub search: LibrarySearchParams,
    /// Optional taxonomic reranking request.
    #[serde(default)]
    pub taxonomy: Option<SearchTaxonomyRequest>,
    /// Optional query identifier mode used for TSV/JSON export convenience fields.
    #[serde(default)]
    pub query_key: Option<SearchQueryKey>,
}

/// Optional taxonomic-reranking inputs for library search.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchTaxonomyRequest {
    /// Taxon label or Wikidata identifier used to resolve the query lineage.
    pub query_text: String,
    /// Human-readable label for the LOTUS metadata source.
    pub lotus_source_label: String,
    /// Inline LOTUS CSV contents.
    #[serde(default)]
    pub lotus_csv_text: Option<String>,
    /// Filesystem path to the LOTUS CSV file.
    #[serde(default)]
    pub lotus_csv_path: Option<String>,
}

/// Spectrum payload embedded in a network artifact.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NetworkSpectrum {
    /// Exported metadata for the spectrum.
    pub meta: SpectrumMetadata,
    /// Raw peak list as `(mz, intensity)` pairs.
    pub peaks: Vec<(f64, f64)>,
}

/// Serialized result of a completed network build.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NetworkArtifact {
    /// Source label used for the input spectra.
    pub source_label: String,
    /// Aggregate parsing statistics for the input file.
    pub parse_stats: ParseStats,
    /// Network-building parameters that produced this artifact.
    pub build: NetworkBuildParams,
    /// Exported spectra used to assemble the network.
    pub spectra: Vec<NetworkSpectrum>,
    /// Final assembled graph.
    pub network: SpectralNetwork,
}

/// Serialized result of a completed library-search job.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchArtifact {
    /// Source label used for the query spectra.
    pub query_source_label: String,
    /// Source label used for the library spectra.
    pub library_source_label: String,
    /// Aggregate parsing statistics for the query file.
    pub query_stats: ParseStats,
    /// Aggregate parsing statistics for the library file.
    pub library_stats: ParseStats,
    /// Search parameters that produced this artifact.
    pub search: LibrarySearchParams,
    /// Optional taxonomic reranking request applied to this search.
    pub taxonomy: Option<SearchTaxonomyRequest>,
    /// Query identifier mode used in the exported TSV/JSON rows.
    pub query_key: SearchQueryKey,
    /// Query metadata in original query order.
    pub query_spectra: Vec<SpectrumMetadata>,
    /// Library metadata in original library order.
    pub library_spectra: Vec<SpectrumMetadata>,
    /// Ranked search result payload.
    pub result: SearchArtifactResult,
    /// Pre-rendered TSV export for convenience.
    pub tsv: String,
}

/// Search result container embedded in a [`SearchArtifact`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchArtifactResult {
    /// Ranked hits after any optional reranking has been applied.
    pub hits: Vec<SearchArtifactHit>,
    /// Number of parsed query spectra.
    pub query_count: usize,
    /// Number of parsed library spectra.
    pub library_count: usize,
    /// Similarity metric used for scoring.
    pub metric: crate::similarity::SimilarityMetric,
    /// Indicates whether taxonomic reranking was applied before final ranking.
    pub taxonomic_reranking_applied: bool,
    /// Resolved query taxon label used for reranking, when present.
    pub taxonomic_query: Option<String>,
}

/// Fully enriched hit row used by JSON and TSV exports.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchArtifactHit {
    /// Zero-based query spectrum index.
    pub query_index: usize,
    /// Zero-based library spectrum index.
    pub library_index: usize,
    /// Final one-based rank after all ranking steps.
    pub rank: usize,
    /// Original one-based spectral rank before taxonomic reranking, when applicable.
    pub rank_before_taxonomy: Option<usize>,
    /// Spectral similarity score.
    pub spectral_score: f64,
    /// Precursor mass deviation in parts per million.
    pub ms1_deviation_ppm: f64,
    /// Taxonomic score contributed by lineage matching.
    pub taxonomic_score: f64,
    /// Combined score used for final ranking when reranking is enabled.
    pub combined_score: f64,
    /// Number of matched fragment peaks reported by the scorer.
    pub matches: usize,
    /// Matched organism name chosen during reranking.
    pub matched_organism_name: Option<String>,
    /// Matched organism Wikidata identifier, if available.
    pub matched_organism_wikidata: Option<String>,
    /// Deepest shared taxonomic rank between query lineage and candidate lineage.
    pub matched_shared_rank: Option<String>,
    /// Short InChIKey used to link spectra to LOTUS biosource metadata.
    pub matched_short_inchikey: Option<String>,
}

/// Minimal health-check payload served by the HTTP API.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: &'static str,
}

/// Response returned when an asynchronous job is created.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobCreatedResponse {
    pub job_id: u64,
    pub status: JobStatus,
}

/// High-level stages emitted by asynchronous and CLI progress reporting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobProgressStage {
    Queued,
    LoadingSpectra,
    LoadingQuery,
    LoadingLibrary,
    LoadingTaxonomy,
    Scoring,
    TaxonomicReranking,
    BuildingNetwork,
    Finalizing,
}

/// Snapshot of progress for a long-running job.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobProgress {
    pub stage: JobProgressStage,
    pub completed: u64,
    pub total: u64,
}

/// Lifecycle state of an asynchronous job.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Running,
    Finished,
    Failed,
}

/// Response payload for polling job state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobStatusResponse {
    pub job_id: u64,
    pub status: JobStatus,
    pub error: Option<String>,
    #[serde(default)]
    pub progress: Option<JobProgress>,
}

/// Discriminated union of possible finished job results.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum MatcherJobResult {
    Network(NetworkArtifact),
    LibrarySearch(SearchArtifact),
}

const fn default_min_peaks() -> usize {
    5
}

const fn default_max_peaks() -> usize {
    1000
}
