//! Core library for spectral-network construction, library matching, export, and service APIs.
//!
//! The crate is organized around a few main workflows:
//! - parse MGF data into in-memory spectrum records
//! - compute pairwise similarity for networking or library search
//! - optionally rerank library hits using taxonomic metadata
//! - export results as JSON/TSV or serve them over HTTP

pub mod api;
pub mod consensus;
pub mod databases;
pub mod export;
pub mod mgf;
pub mod model;
pub mod network;
pub mod search;
#[cfg(not(target_arch = "wasm32"))]
pub mod server;
pub mod similarity;
pub mod taxonomy;

/// API request/response types shared by the CLI, server, and tests.
pub use api::{
    ConsensusAnnotation, ConsensusArtifact, ConsensusArtifactResult, ConsensusClass,
    ConsensusInputSummary, ConsensusMergeParams, ConsensusQueryResult, ConsensusSupportHit,
    HealthResponse, JobCreatedResponse, JobProgress, JobProgressStage, JobStatus,
    JobStatusResponse, MatcherJobResult, NetworkArtifact, NetworkBuildParams, NetworkRequest,
    NetworkSpectrum, ParseConfig, SearchArtifact, SearchArtifactHit, SearchArtifactResult,
    SearchRequest, SearchTaxonomyRequest,
};
/// Post-search consensus fusion across two library-search artifacts.
pub use consensus::merge_search_artifacts;
/// Curated downloadable spectral database metadata and helpers.
pub use databases::{SpectralDatabase, resolve_spectral_database, spectral_databases};
#[cfg(not(target_arch = "wasm32"))]
pub use databases::download_spectral_database;
#[cfg(target_arch = "wasm32")]
pub use export::download_tsv_file;
pub use export::{SearchQueryKey, export_consensus_tsv, export_search_json, export_search_tsv};
#[cfg(not(target_arch = "wasm32"))]
pub use export::{save_json_to_path, save_tsv_to_path};
/// Parse inline MGF bytes into a loaded spectrum collection.
pub use mgf::load_mgf_bytes;
#[cfg(target_arch = "wasm32")]
pub use mgf::load_mgf_file_for_wasm;
#[cfg(not(target_arch = "wasm32"))]
pub use mgf::{NativeLoadHandle, NativeLoadMessage, load_mgf_path, start_native_mgf_load};
/// Shared spectral-model types used across parsing, searching, and exporting.
pub use model::{
    CandidateHit, HitLike, LoadedSpectra, ParseStats, SearchResult, SpectrumCollection,
    SpectrumMetadata, SpectrumRecord,
};
/// Network graph types and the core graph assembly helper.
pub use network::{ComponentSelection, NetworkEdge, NetworkNode, SpectralNetwork, build_network};
/// Search entrypoints and types for direct and progressive search workflows.
pub use search::{
    IncrementalSearchState, IncrementalSearchStep, LibrarySearchParams, NativeSearchHandle,
    SearchMessage, build_network_artifact, run_search_request, search_library, start_native_search,
    total_search_pairs,
};
#[cfg(not(target_arch = "wasm32"))]
pub use search::{build_network_artifact_with_progress, run_search_request_with_progress};
#[cfg(not(target_arch = "wasm32"))]
pub use server::serve;
/// Similarity configuration and scorer construction helpers.
pub use similarity::{
    ComputeParams, MetricScorer, SimilarityMetric, preprocess_spectra_for_metric,
};
