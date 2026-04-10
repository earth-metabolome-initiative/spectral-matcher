//! Serializable request, artifact, and job-status types shared across the matcher interfaces.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::export::SearchQueryKey;
use crate::model::{ParseStats, SpectrumMetadata};
use crate::network::SpectralNetwork;
use crate::search::LibrarySearchParams;
use crate::similarity::ComputeParams;

/// Generic parsing limits applied while reading MGF data.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParseConfig {
    /// Exact MGF header name reused verbatim as the canonical exported spectrum identifier.
    #[serde(default = "default_identifier_header")]
    pub identifier: String,
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
            identifier: default_identifier_header(),
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
    /// Minimum number of matched fragment peaks required to keep an edge.
    #[serde(default = "default_network_min_matched_peaks")]
    pub min_matched_peaks: usize,
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

/// Parameters controlling post-search consensus fusion across multiple library outputs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConsensusMergeParams {
    /// Maximum rank depth retained from each input artifact before consensus fusion.
    #[serde(default = "default_consensus_top_k_per_library")]
    pub top_k_per_library: usize,
    /// Reciprocal-rank-fusion offset controlling how sharply top ranks are favored.
    #[serde(default = "default_consensus_rrf_k")]
    pub rrf_k: f64,
    /// Additive bonus applied when a consensus group is supported by both inputs.
    #[serde(default = "default_consensus_bonus")]
    pub consensus_bonus: f64,
    /// Relative weight assigned to the left/first input during fusion.
    #[serde(default = "default_consensus_weight")]
    pub left_weight: f64,
    /// Relative weight assigned to the right/second input during fusion.
    #[serde(default = "default_consensus_weight")]
    pub right_weight: f64,
}

impl Default for ConsensusMergeParams {
    fn default() -> Self {
        Self {
            top_k_per_library: default_consensus_top_k_per_library(),
            rrf_k: default_consensus_rrf_k(),
            consensus_bonus: default_consensus_bonus(),
            left_weight: default_consensus_weight(),
            right_weight: default_consensus_weight(),
        }
    }
}

/// Summary of a single input artifact participating in cross-library consensus fusion.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConsensusInputSummary {
    /// Short stable input name used in provenance and export columns.
    pub name: String,
    /// Query source label shared across merged inputs.
    pub query_source_label: String,
    /// Library source label for this input artifact.
    pub library_source_label: String,
    /// Search parameters used to generate the input artifact.
    pub search: LibrarySearchParams,
    /// Similarity metric used by the originating search.
    pub metric: crate::similarity::SimilarityMetric,
    /// Indicates whether taxonomic reranking was applied in the originating search.
    pub taxonomic_reranking_applied: bool,
    /// Taxonomic query used by the originating search, when present.
    pub taxonomic_query: Option<String>,
}

/// Serialized result of a merged cross-library consensus job.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConsensusArtifact {
    /// Shared source label for the merged query spectra.
    pub query_source_label: String,
    /// Query identifier mode used in TSV/JSON convenience exports.
    pub query_key: SearchQueryKey,
    /// Query metadata in original query order.
    pub query_spectra: Vec<SpectrumMetadata>,
    /// Merge parameters used to produce consensus annotations.
    pub merge: ConsensusMergeParams,
    /// Input artifact summaries participating in the merge.
    pub inputs: Vec<ConsensusInputSummary>,
    /// One consensus result per query spectrum.
    pub result: ConsensusArtifactResult,
    /// Pre-rendered TSV export for convenience.
    pub tsv: String,
}

/// Query-oriented consensus results embedded in a [`ConsensusArtifact`].
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ConsensusArtifactResult {
    /// One merged row per query spectrum.
    pub queries: Vec<ConsensusQueryResult>,
    /// Number of query spectra considered during merging.
    pub query_count: usize,
    /// Number of query spectra receiving a merged annotation.
    pub annotated_query_count: usize,
}

/// Consensus result for a single query spectrum.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ConsensusQueryResult {
    /// Zero-based query spectrum index.
    pub query_index: usize,
    /// Winning merged annotation for the query, when any candidate survives fusion.
    pub annotation: Option<ConsensusAnnotation>,
}

/// High-level class describing how a consensus annotation was supported.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsensusClass {
    /// Only one input artifact supported the final annotation.
    Singleton,
    /// Both inputs agreed on the same exact full InChIKey.
    CrossLibraryExact,
    /// Both inputs agreed only at the short-InChIKey/scaffold level.
    CrossLibraryShortInchikey,
}

impl Default for ConsensusClass {
    fn default() -> Self {
        Self::Singleton
    }
}

/// Best supporting hit retained from one input artifact for a consensus annotation.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ConsensusSupportHit {
    /// Short stable input name such as `gnps` or `isdb`.
    pub input_name: String,
    /// Source label of the library that produced this hit.
    pub library_source_label: String,
    /// Zero-based library spectrum index in the originating artifact.
    pub library_index: usize,
    /// Final one-based rank in the originating artifact.
    pub rank: usize,
    /// Original spectral rank before taxonomy reranking, when available.
    pub rank_before_taxonomy: Option<usize>,
    /// Spectral similarity score from the originating artifact.
    pub spectral_score: f64,
    /// Taxonomic reranking contribution from the originating artifact.
    pub taxonomic_score: f64,
    /// Final combined score from the originating artifact.
    pub combined_score: f64,
    /// Number of matched fragment peaks in the originating artifact.
    pub matches: usize,
    /// Library precursor m/z associated with the support hit.
    pub precursor_mz: f64,
    /// MS1 deviation reported for the support hit.
    pub ms1_deviation_ppm: f64,
    /// Human-readable preferred name for the support hit.
    pub raw_name: String,
    /// Matched organism name, when taxonomy reranking was applied.
    pub organism_name: Option<String>,
    /// Matched organism Wikidata identifier, when available.
    pub organism_wikidata: Option<String>,
    /// Deepest shared taxonomic rank, when available.
    pub shared_rank: Option<String>,
    /// Short InChIKey used for grouping consensus candidates.
    pub short_inchikey: Option<String>,
    /// Full InChIKey recovered from the representative library metadata, when available.
    pub full_inchikey: Option<String>,
    /// Retained source attributes from the representative library spectrum.
    pub attributes: BTreeMap<String, String>,
}

/// Winning consensus annotation and provenance for a query spectrum.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ConsensusAnnotation {
    /// Shared grouping key used during consensus fusion, typically a short InChIKey.
    pub consensus_key: Option<String>,
    /// Final consensus score after reciprocal-rank fusion and consensus bonus.
    pub consensus_score: f64,
    /// Whether the annotation is singleton support, exact cross-library consensus, or scaffold-level consensus.
    pub consensus_class: ConsensusClass,
    /// Indicates whether all supporting hits resolved to one exact full InChIKey.
    pub exact_structure_consensus: bool,
    /// Input names that contributed evidence to the final annotation.
    pub support_libraries: Vec<String>,
    /// Number of unique input libraries supporting the final annotation.
    pub support_count: usize,
    /// Number of raw hits collapsed into the final consensus group before per-input deduplication.
    pub support_hit_count: usize,
    /// Best retained rank per input library.
    pub best_rank_by_input: BTreeMap<String, usize>,
    /// Best retained spectral score per input library.
    pub best_spectral_score_by_input: BTreeMap<String, f64>,
    /// Best retained taxonomic score per input library.
    pub best_taxonomic_score_by_input: BTreeMap<String, f64>,
    /// Best retained combined score per input library.
    pub best_combined_score_by_input: BTreeMap<String, f64>,
    /// Best retained fragment-match count per input library.
    pub best_matches_by_input: BTreeMap<String, usize>,
    /// Input name that supplied the representative structure shown to downstream consumers.
    pub representative_input_name: String,
    /// Library source label for the representative structure.
    pub representative_library_source_label: String,
    /// Zero-based library index of the representative structure.
    pub representative_library_index: usize,
    /// Final one-based rank of the representative support hit.
    pub representative_rank: usize,
    /// Spectral rank before taxonomy reranking for the representative hit, when available.
    pub representative_rank_before_taxonomy: Option<usize>,
    /// Spectral similarity score for the representative hit.
    pub representative_spectral_score: f64,
    /// Taxonomic reranking contribution for the representative hit.
    pub representative_taxonomic_score: f64,
    /// Final combined score for the representative hit.
    pub representative_combined_score: f64,
    /// Fragment-match count for the representative hit.
    pub representative_matches: usize,
    /// Precursor m/z for the representative structure.
    pub representative_precursor_mz: f64,
    /// MS1 deviation for the representative structure.
    pub representative_ms1_deviation_ppm: f64,
    /// Preferred name of the representative structure.
    pub representative_raw_name: String,
    /// Matched organism name of the representative structure, when available.
    pub representative_organism_name: Option<String>,
    /// Matched organism Wikidata identifier of the representative structure, when available.
    pub representative_organism_wikidata: Option<String>,
    /// Deepest shared taxonomic rank for the representative structure, when available.
    pub representative_shared_rank: Option<String>,
    /// Short InChIKey for the representative structure, when available.
    pub representative_short_inchikey: Option<String>,
    /// Full InChIKey for the representative structure, when available.
    pub representative_full_inchikey: Option<String>,
    /// Retained source attributes for the representative structure.
    pub representative_attributes: BTreeMap<String, String>,
    /// Best supporting hit retained from each contributing input artifact.
    pub support_hits: Vec<ConsensusSupportHit>,
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

fn default_identifier_header() -> String {
    "FEATURE_ID".to_string()
}

const fn default_consensus_top_k_per_library() -> usize {
    5
}

const fn default_network_min_matched_peaks() -> usize {
    1
}

const fn default_consensus_rrf_k() -> f64 {
    10.0
}

const fn default_consensus_bonus() -> f64 {
    0.05
}

const fn default_consensus_weight() -> f64 {
    1.0
}
