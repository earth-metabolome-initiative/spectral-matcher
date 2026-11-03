//! Search and network workflows built on top of parsed spectra and similarity scoring.

#[cfg(not(target_arch = "wasm32"))]
use std::cmp::Reverse;
#[cfg(not(target_arch = "wasm32"))]
use std::collections::{BinaryHeap, HashMap};
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::{LazyLock, OnceLock};
#[cfg(not(target_arch = "wasm32"))]
use std::time::{Duration, UNIX_EPOCH};

#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(not(target_arch = "wasm32"))]
use crate::api::JobProgressStage;
use crate::api::{
    MissingPrecursorMz, NetworkArtifact, NetworkRequest, NetworkSpectrum, ParseConfig,
    SearchArtifact, SearchArtifactHit, SearchArtifactResult, SearchRequest,
};
use crate::export::{SearchQueryKey, export_search_tsv};
use crate::mgf::load_mgf_bytes_with_policy;
#[cfg(not(target_arch = "wasm32"))]
use crate::mgf::{NativeLoadMessage, start_native_mgf_load_with_policy};
use crate::model::{CandidateHit, LoadedSpectra, SearchResult, SpectrumRecord};
#[cfg(target_arch = "wasm32")]
use crate::network::PairScore;
#[cfg(target_arch = "wasm32")]
use crate::network::build_network;
#[cfg(not(target_arch = "wasm32"))]
use crate::network::{SelectedNeighbor, build_network_from_selected_neighbors};
#[cfg(not(target_arch = "wasm32"))]
use crate::similarity::MetricIndexSearchState;
use crate::similarity::{ComputeParams, MetricIndex, MetricScorer, preprocess_spectra_for_metric};
use crate::taxonomy::{LotusMetadataIndex, ResolvedLotusQuery, short_inchikey_from_record};

/// Parameters controlling library-search filtering and final ranking.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LibrarySearchParams {
    /// Fragment-level similarity parameters.
    pub compute: ComputeParams,
    /// Maximum allowed precursor m/z deviation in Dalton for candidate prefiltering.
    pub precursor_mz_tolerance: f64,
    /// Minimum number of matched fragment peaks required for a hit to survive.
    pub min_matched_peaks: usize,
    /// Minimum spectral similarity score required for a hit to survive.
    pub min_similarity_threshold: f64,
    /// Maximum number of final hits retained per query after any reranking.
    pub top_n: usize,
}

/// Messages produced by the background native search worker.
#[derive(Debug)]
pub enum SearchMessage {
    Finished(SearchResult),
    Cancelled,
    Failed(String),
}

/// Handle used to poll progress from a background native search.
pub struct NativeSearchHandle {
    total: usize,
    done: Arc<AtomicUsize>,
    cancel: Arc<AtomicBool>,
    rx: Receiver<SearchMessage>,
}

impl NativeSearchHandle {
    /// Returns the total number of query/library pairs to evaluate.
    pub fn total(&self) -> usize {
        self.total
    }

    /// Returns the number of pairs processed so far.
    pub fn done(&self) -> usize {
        self.done.load(Ordering::Relaxed)
    }

    /// Requests cancellation of the background search.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Non-blocking poll for a completed search result.
    pub fn try_recv(&self) -> Option<SearchMessage> {
        self.rx.try_recv().ok()
    }
}

#[derive(Clone, Debug, PartialEq)]
struct SearchCandidate {
    query_index: usize,
    library_index: usize,
    rank_before_taxonomy: usize,
    spectral_score: f64,
    taxonomic_score: f64,
    combined_score: f64,
    matches: usize,
    matched_organism_name: Option<String>,
    matched_organism_wikidata: Option<String>,
    matched_shared_rank: Option<String>,
    matched_short_inchikey: Option<String>,
}

#[derive(Clone, Debug)]
struct SearchTaxonomyConfig {
    #[cfg(not(target_arch = "wasm32"))]
    lotus: Arc<LotusMetadataIndex>,
    #[cfg(target_arch = "wasm32")]
    lotus: LotusMetadataIndex,
    query: ResolvedLotusQuery,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, Eq, PartialEq)]
struct FileFingerprint {
    len: u64,
    modified: Option<(u64, u32)>,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
struct CachedMgf {
    fingerprint: FileFingerprint,
    identifier: String,
    min_peaks: usize,
    max_peaks: usize,
    missing_precursor_policy: MissingPrecursorMz,
    loaded: Arc<LoadedSpectra>,
}

#[cfg(not(target_arch = "wasm32"))]
struct PreparedLibrary {
    loaded: Arc<LoadedSpectra>,
    processed: Vec<SpectrumRecord>,
    index: OnceLock<Result<Option<MetricIndex>, String>>,
    search_states: OnceLock<IndexedSearchScratch>,
}

/// Each Rayon worker keeps its own index-sized scratch, even when Rayon splits
/// the query iterator into many tasks. The final slot handles callers outside
/// the pool and workers from a larger, separately configured pool.
#[cfg(not(target_arch = "wasm32"))]
struct IndexedSearchScratch {
    workers: Vec<Mutex<MetricIndexSearchState>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl IndexedSearchScratch {
    fn new(index: &MetricIndex) -> Self {
        let workers = (0..rayon::current_num_threads() + 1)
            .map(|_| Mutex::new(index.new_search_state()))
            .collect();
        Self { workers }
    }

    fn for_current_worker(&self) -> &Mutex<MetricIndexSearchState> {
        let fallback = self.workers.len() - 1;
        let slot = rayon::current_thread_index()
            .filter(|&index| index < fallback)
            .unwrap_or(fallback);
        &self.workers[slot]
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct CachedPreparedLibrary {
    path: PathBuf,
    fingerprint: FileFingerprint,
    identifier: String,
    min_peaks: usize,
    max_peaks: usize,
    missing_precursor_policy: MissingPrecursorMz,
    compute: ComputeParams,
    precursor_mz_tolerance: f64,
    prepared: Arc<PreparedLibrary>,
}

#[cfg(not(target_arch = "wasm32"))]
struct CachedLotus {
    path: PathBuf,
    fingerprint: FileFingerprint,
    index: Arc<LotusMetadataIndex>,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug)]
struct RankedNeighbor {
    neighbor: usize,
    score: f64,
    matches: usize,
}

#[cfg(not(target_arch = "wasm32"))]
impl PartialEq for RankedNeighbor {
    fn eq(&self, other: &Self) -> bool {
        self.neighbor == other.neighbor
            && self.matches == other.matches
            && self.score.to_bits() == other.score.to_bits()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Eq for RankedNeighbor {}

#[cfg(not(target_arch = "wasm32"))]
impl PartialOrd for RankedNeighbor {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Ord for RankedNeighbor {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.score
            .total_cmp(&other.score)
            .then_with(|| other.neighbor.cmp(&self.neighbor))
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn mgf_cache() -> &'static Mutex<HashMap<PathBuf, CachedMgf>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, CachedMgf>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(not(target_arch = "wasm32"))]
fn prepared_library_cache() -> &'static Mutex<Option<CachedPreparedLibrary>> {
    static CACHE: LazyLock<Mutex<Option<CachedPreparedLibrary>>> =
        LazyLock::new(|| Mutex::new(None));
    &CACHE
}

#[cfg(not(target_arch = "wasm32"))]
fn same_compute(left: ComputeParams, right: ComputeParams) -> bool {
    left.metric == right.metric
        && left.fragment_mz_tolerance.to_bits() == right.fragment_mz_tolerance.to_bits()
        && left.mz_power.to_bits() == right.mz_power.to_bits()
        && left.intensity_power.to_bits() == right.intensity_power.to_bits()
        && left.top_n_peaks == right.top_n_peaks
}

#[cfg(not(target_arch = "wasm32"))]
fn lotus_cache() -> &'static Mutex<Option<CachedLotus>> {
    static CACHE: LazyLock<Mutex<Option<CachedLotus>>> = LazyLock::new(|| Mutex::new(None));
    &CACHE
}

fn network_missing_precursor_policy(parse: &ParseConfig) -> MissingPrecursorMz {
    match parse.missing_precursor_mz {
        MissingPrecursorMz::Reject => MissingPrecursorMz::UseZero,
        policy => policy,
    }
}

/// Returns the total number of query/library comparisons for a search job.
pub fn total_search_pairs(query_count: usize, library_count: usize) -> usize {
    query_count.saturating_mul(library_count)
}

/// Returns the number of unique pair slots in an upper-triangular all-vs-all matrix.
pub fn total_pairs(count: usize) -> usize {
    count.saturating_mul(count.saturating_add(1)) / 2
}

#[cfg(not(target_arch = "wasm32"))]
fn total_network_pairs(count: usize) -> usize {
    count.saturating_mul(count.saturating_sub(1)) / 2
}

/// Loads LOTUS metadata and resolves the requested query lineage.
fn load_search_taxonomy_config(
    request: &crate::api::SearchTaxonomyRequest,
) -> Result<SearchTaxonomyConfig, String> {
    let lotus = if let Some(text) = request.lotus_csv_text.as_deref() {
        #[cfg(not(target_arch = "wasm32"))]
        {
            Arc::new(crate::taxonomy::load_lotus_bytes(text.as_bytes())?)
        }
        #[cfg(target_arch = "wasm32")]
        {
            crate::taxonomy::load_lotus_bytes(text.as_bytes())?
        }
    } else if let Some(_path) = request.lotus_csv_path.as_deref() {
        #[cfg(not(target_arch = "wasm32"))]
        {
            load_lotus_path_cached(std::path::Path::new(_path))?
        }
        #[cfg(target_arch = "wasm32")]
        {
            return Err("LOTUS CSV path loading is unavailable on wasm".to_string());
        }
    } else {
        return Err(format!(
            "LOTUS source '{}' is missing CSV text and path",
            request.lotus_source_label
        ));
    };

    let query = lotus
        .resolve_query_lineage(&request.query_text)
        .ok_or_else(|| {
            format!(
                "Biosource not found in LOTUS: {}",
                request.query_text.trim()
            )
        })?;

    Ok(SearchTaxonomyConfig { lotus, query })
}

/// Shares the immutable LOTUS index across file-based native searches without
/// retaining an unbounded collection of large reference datasets.
#[cfg(not(target_arch = "wasm32"))]
fn load_lotus_path_cached(path: &Path) -> Result<Arc<LotusMetadataIndex>, String> {
    let canonical = std::fs::canonicalize(path)
        .map_err(|err| format!("cannot resolve {}: {err}", path.display()))?;
    let fingerprint = file_fingerprint(&canonical)?;
    // Hold the lock through the first load so concurrent requests for this path
    // cannot each parse a separate copy of the large reference CSV.
    let mut cached = lotus_cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(entry) = cached.as_ref()
        && entry.path == canonical
        && entry.fingerprint == fingerprint
    {
        return Ok(Arc::clone(&entry.index));
    }
    let index = Arc::new(crate::taxonomy::load_lotus_path(&canonical)?);
    *cached = Some(CachedLotus {
        path: canonical,
        fingerprint,
        index: Arc::clone(&index),
    });
    Ok(index)
}

/// Expands the intermediate candidate budget when taxonomy reranking is enabled.
fn effective_search_params(
    params: &LibrarySearchParams,
    library_count: usize,
    taxonomy_enabled: bool,
) -> LibrarySearchParams {
    let mut effective = params.clone();
    if taxonomy_enabled {
        effective.top_n = library_count.max(1);
    }
    effective
}

/// Runs a direct library search without artifact enrichment or progress callbacks.
pub fn search_library(
    queries: Vec<SpectrumRecord>,
    library: Vec<SpectrumRecord>,
    params: LibrarySearchParams,
) -> Result<SearchResult, String> {
    let query_count = queries.len();
    let library_count = library.len();
    let queries = preprocess_spectra_for_metric(queries, params.compute)?;
    let library = preprocess_spectra_for_metric(library, params.compute)?;
    #[cfg(not(target_arch = "wasm32"))]
    {
        let total = total_search_pairs(query_count, library_count);
        let done = Arc::new(AtomicUsize::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let hits = if let Some(index) = MetricIndex::new(params.compute, &library).transpose()? {
            let scratch = IndexedSearchScratch::new(&index);
            score_query_library_pairs_indexed(
                &queries,
                &library,
                &params,
                &index,
                &scratch,
                total,
                done,
                cancel,
                &|_, _, _| {},
                &|| false,
            )?
        } else {
            let scorer = MetricScorer::new(params.compute)?;
            score_query_library_pairs(
                &queries,
                &library,
                &params,
                &scorer,
                total,
                done,
                cancel,
                &|_, _, _| {},
                &|| false,
            )?
        }
        .ok_or_else(|| "search was unexpectedly cancelled".to_string())?;
        Ok(SearchResult {
            hits,
            query_count,
            library_count,
            metric: params.compute.metric,
        })
    }

    #[cfg(target_arch = "wasm32")]
    {
        let scorer = MetricScorer::new(params.compute)?;
        let candidates = search_candidates(&queries, &library, &params, &scorer)?;
        Ok(SearchResult {
            hits: finalize_search_candidates(candidates, params.top_n),
            query_count,
            library_count,
            metric: params.compute.metric,
        })
    }
}

/// Builds a network artifact from a request without surfacing progress.
pub fn build_network_artifact(request: NetworkRequest) -> Result<NetworkArtifact, String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        return build_network_artifact_with_progress(request, |_, _, _| {}, || false);
    }

    #[cfg(target_arch = "wasm32")]
    {
        let missing_precursor_policy = network_missing_precursor_policy(&request.parse);
        let loaded = load_request_spectra(
            &request.source_label,
            request.mgf_text.as_deref(),
            request.mgf_path.as_deref(),
            &request.parse.identifier,
            request.parse.min_peaks,
            request.parse.max_peaks,
            missing_precursor_policy,
        )?;
        let spectra = preprocess_spectra_for_metric(loaded.spectra.clone(), request.build.compute)?;
        let scorer = MetricScorer::new(request.build.compute)?;
        #[cfg(not(target_arch = "wasm32"))]
        let pair_scores = score_all_pairs_parallel(&spectra, &scorer)?;
        #[cfg(target_arch = "wasm32")]
        let pair_scores =
            score_all_pairs(&spectra, &scorer, request.build.min_matched_peaks.max(1))?;
        let metas = loaded
            .spectra
            .iter()
            .map(|record| record.meta.clone())
            .collect::<Vec<_>>();
        let network = build_network(
            &metas,
            &pair_scores,
            request.build.threshold.clamp(0.0, 1.0),
            request.build.top_k.max(1),
        );
        let spectra = loaded
            .spectra
            .iter()
            .map(|record| NetworkSpectrum {
                meta: record.meta.clone(),
                peaks: record.peaks.as_ref().clone(),
            })
            .collect();
        Ok(NetworkArtifact {
            source_label: loaded.source_label,
            parse_stats: loaded.stats,
            build: request.build,
            spectra,
            network,
        })
    }
}

/// Runs a search request without surfacing progress.
pub fn run_search_request(request: SearchRequest) -> Result<SearchArtifact, String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        return run_search_request_with_progress(request, |_, _, _| {}, || false);
    }

    #[cfg(target_arch = "wasm32")]
    {
        let queries = load_request_spectra(
            &request.query_source_label,
            request.query_mgf_text.as_deref(),
            request.query_mgf_path.as_deref(),
            request.parse.query_identifier(),
            request.parse.min_peaks,
            request.parse.max_peaks,
            request.parse.missing_precursor_mz,
        )?;
        let library = load_request_spectra(
            &request.library_source_label,
            request.library_mgf_text.as_deref(),
            request.library_mgf_path.as_deref(),
            request.parse.library_identifier(),
            request.parse.min_peaks,
            request.parse.max_peaks,
            request.parse.missing_precursor_mz,
        )?;
        let taxonomy = request
            .taxonomy
            .as_ref()
            .map(load_search_taxonomy_config)
            .transpose()?;
        let search_params =
            effective_search_params(&request.search, library.spectra.len(), taxonomy.is_some());
        let query_key = request.query_key.unwrap_or(SearchQueryKey::SpectrumId);
        let result = search_library(
            queries.spectra.clone(),
            library.spectra.clone(),
            search_params,
        )?;
        let enriched = build_search_artifact_result(
            result,
            &queries.spectra,
            &library.spectra,
            taxonomy.as_ref(),
            request.search.top_n,
        );
        let tsv = export_search_tsv(&enriched, &queries.spectra, &library.spectra, query_key);
        Ok(SearchArtifact {
            query_source_label: queries.source_label,
            library_source_label: library.source_label,
            query_stats: queries.stats,
            library_stats: library.stats,
            search: request.search,
            taxonomy: request.taxonomy,
            query_key,
            query_spectra: queries
                .spectra
                .iter()
                .map(|record| record.meta.clone())
                .collect(),
            library_spectra: library
                .spectra
                .iter()
                .map(|record| record.meta.clone())
                .collect(),
            result: enriched,
            tsv,
        })
    }
}

/// Builds a network artifact while reporting stage progress.
#[cfg(not(target_arch = "wasm32"))]
pub fn build_network_artifact_with_progress<F>(
    request: NetworkRequest,
    on_progress: F,
    is_cancelled: impl Fn() -> bool + Send + Sync,
) -> Result<NetworkArtifact, String>
where
    F: Fn(JobProgressStage, u64, u64) + Send + Sync,
{
    let missing_precursor_policy = network_missing_precursor_policy(&request.parse);
    let loaded = load_request_spectra_with_progress(
        JobProgressStage::LoadingSpectra,
        &request.source_label,
        request.mgf_text.as_deref(),
        request.mgf_path.as_deref(),
        &request.parse.identifier,
        request.parse.min_peaks,
        request.parse.max_peaks,
        missing_precursor_policy,
        &on_progress,
    )?;

    let spectra = preprocess_spectra_for_metric(loaded.spectra.clone(), request.build.compute)?;
    let total = total_network_pairs(spectra.len()) as u64;
    let done = Arc::new(AtomicUsize::new(0));
    on_progress(JobProgressStage::Scoring, 0, total);
    let selected_neighbors =
        if let Some(index) = MetricIndex::new(request.build.compute, &spectra).transpose()? {
            score_network_neighbors_indexed_parallel(
                &spectra,
                &index,
                request.build.threshold.clamp(0.0, 1.0),
                request.build.min_matched_peaks.max(1),
                request.build.top_k.max(1),
                total as usize,
                Arc::clone(&done),
                &on_progress,
                &is_cancelled,
            )?
        } else {
            let scorer = MetricScorer::new(request.build.compute)?;
            score_network_neighbors_parallel(
                &spectra,
                &scorer,
                request.build.threshold.clamp(0.0, 1.0),
                request.build.min_matched_peaks.max(1),
                request.build.top_k.max(1),
                total as usize,
                Arc::clone(&done),
                &on_progress,
                &is_cancelled,
            )?
        };
    on_progress(
        JobProgressStage::Scoring,
        done.load(Ordering::Relaxed) as u64,
        total,
    );
    on_progress(JobProgressStage::BuildingNetwork, 0, 1);

    let metas = loaded
        .spectra
        .iter()
        .map(|record| record.meta.clone())
        .collect::<Vec<_>>();
    let network = build_network_from_selected_neighbors(&metas, &selected_neighbors);
    let spectra = loaded
        .spectra
        .iter()
        .map(|record| NetworkSpectrum {
            meta: record.meta.clone(),
            peaks: record.peaks.as_ref().clone(),
        })
        .collect();
    on_progress(JobProgressStage::Finalizing, 1, 1);
    Ok(NetworkArtifact {
        source_label: loaded.source_label,
        parse_stats: loaded.stats,
        build: request.build,
        spectra,
        network,
    })
}

/// Runs a search request while reporting stage progress.
#[cfg(not(target_arch = "wasm32"))]
pub fn run_search_request_with_progress<F>(
    request: SearchRequest,
    on_progress: F,
    is_cancelled: impl Fn() -> bool + Send + Sync,
) -> Result<SearchArtifact, String>
where
    F: Fn(JobProgressStage, u64, u64) + Send + Sync,
{
    let queries = load_request_spectra_with_progress(
        JobProgressStage::LoadingQuery,
        &request.query_source_label,
        request.query_mgf_text.as_deref(),
        request.query_mgf_path.as_deref(),
        request.parse.query_identifier(),
        request.parse.min_peaks,
        request.parse.max_peaks,
        request.parse.missing_precursor_mz,
        &on_progress,
    )?;

    let prepared_library = load_prepared_library_with_progress(&request, &on_progress)?;
    let library = prepared_library.loaded.as_ref();
    let library_processed = &prepared_library.processed;

    let queries_processed =
        preprocess_spectra_for_metric(queries.spectra.clone(), request.search.compute)?;
    // The prepared library and any Flash index are reused across file-based searches.
    let taxonomy = if let Some(taxonomy_request) = request.taxonomy.as_ref() {
        on_progress(JobProgressStage::LoadingTaxonomy, 0, 1);
        let loaded = load_search_taxonomy_config(taxonomy_request)?;
        on_progress(JobProgressStage::LoadingTaxonomy, 1, 1);
        Some(loaded)
    } else {
        None
    };
    let search_params =
        effective_search_params(&request.search, library_processed.len(), taxonomy.is_some());
    let total = total_search_pairs(queries_processed.len(), library_processed.len());
    let done = Arc::new(AtomicUsize::new(0));
    let cancel = Arc::new(AtomicBool::new(false));
    on_progress(JobProgressStage::Scoring, 0, total as u64);
    let index = prepared_library
        .index
        .get_or_init(|| {
            MetricIndex::new_for_search(
                search_params.compute,
                library_processed,
                search_params.precursor_mz_tolerance,
            )
            .transpose()
        })
        .as_ref()
        .map_err(Clone::clone)?;
    let hits = if let Some(index) = index.as_ref() {
        let scratch = prepared_library
            .search_states
            .get_or_init(|| IndexedSearchScratch::new(index));
        score_query_library_pairs_indexed(
            &queries_processed,
            library_processed,
            &search_params,
            index,
            scratch,
            total,
            Arc::clone(&done),
            cancel,
            &on_progress,
            &is_cancelled,
        )?
    } else {
        let scorer = MetricScorer::new(search_params.compute)?;
        score_query_library_pairs(
            &queries_processed,
            &library_processed,
            &search_params,
            &scorer,
            total,
            Arc::clone(&done),
            cancel,
            &on_progress,
            &is_cancelled,
        )?
    }
    .ok_or_else(|| "search was unexpectedly cancelled".to_string())?;
    on_progress(
        JobProgressStage::Scoring,
        done.load(Ordering::Relaxed) as u64,
        total as u64,
    );

    on_progress(JobProgressStage::Finalizing, 0, 1);
    let query_key = request.query_key.unwrap_or(SearchQueryKey::SpectrumId);
    let result = SearchResult {
        hits,
        query_count: queries.spectra.len(),
        library_count: library.spectra.len(),
        metric: request.search.compute.metric,
    };
    if taxonomy.is_some() {
        on_progress(JobProgressStage::TaxonomicReranking, 0, 1);
    }
    let enriched = build_search_artifact_result(
        result,
        &queries.spectra,
        &library.spectra,
        taxonomy.as_ref(),
        request.search.top_n,
    );
    if taxonomy.is_some() {
        on_progress(JobProgressStage::TaxonomicReranking, 1, 1);
    }
    let tsv = export_search_tsv(&enriched, &queries.spectra, &library.spectra, query_key);
    on_progress(JobProgressStage::Finalizing, 1, 1);
    Ok(SearchArtifact {
        query_source_label: queries.source_label,
        library_source_label: library.source_label.clone(),
        query_stats: queries.stats,
        library_stats: library.stats,
        search: request.search,
        taxonomy: request.taxonomy,
        query_key,
        query_spectra: queries
            .spectra
            .iter()
            .map(|record| record.meta.clone())
            .collect(),
        library_spectra: library
            .spectra
            .iter()
            .map(|record| record.meta.clone())
            .collect(),
        result: enriched,
        tsv,
    })
}

#[cfg(target_arch = "wasm32")]
fn load_request_spectra(
    source_label: &str,
    mgf_text: Option<&str>,
    mgf_path: Option<&str>,
    identifier: &str,
    min_peaks: usize,
    max_peaks: usize,
    missing_precursor_policy: MissingPrecursorMz,
) -> Result<LoadedSpectra, String> {
    if let Some(text) = mgf_text {
        return load_mgf_bytes_with_policy(
            source_label,
            text.as_bytes(),
            identifier,
            min_peaks,
            max_peaks,
            missing_precursor_policy,
        );
    }

    if mgf_path.is_some() {
        return Err(
            "MGF path requests are unavailable on wasm; provide inline MGF text".to_string(),
        );
    }

    Err("request did not provide an MGF source".to_string())
}

/// Reuses one file-backed, metric-specific library across search requests.
/// Only one large prepared library is retained; changing the file, parse
/// policy or scoring parameters replaces it.
#[cfg(not(target_arch = "wasm32"))]
fn load_prepared_library_with_progress<F>(
    request: &SearchRequest,
    on_progress: &F,
) -> Result<Arc<PreparedLibrary>, String>
where
    F: Fn(JobProgressStage, u64, u64) + Sync,
{
    let identifier = request.parse.library_identifier();
    if let Some(path) = request.library_mgf_path.as_deref() {
        let canonical =
            std::fs::canonicalize(path).map_err(|err| format!("cannot resolve {path}: {err}"))?;
        let fingerprint = file_fingerprint(&canonical)?;
        let mut cached = prepared_library_cache()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(entry) = cached.as_ref()
            && entry.path == canonical
            && entry.fingerprint == fingerprint
            && entry.identifier == identifier
            && entry.min_peaks == request.parse.min_peaks
            && entry.max_peaks == request.parse.max_peaks
            && entry.missing_precursor_policy == request.parse.missing_precursor_mz
            && same_compute(entry.compute, request.search.compute)
            && entry.precursor_mz_tolerance.to_bits()
                == request.search.precursor_mz_tolerance.to_bits()
        {
            let total = fingerprint.len.max(1);
            on_progress(JobProgressStage::LoadingLibrary, total, total);
            return Ok(Arc::clone(&entry.prepared));
        }

        let loaded = load_mgf_path_cached_with_progress(
            &canonical,
            identifier,
            request.parse.min_peaks,
            request.parse.max_peaks,
            request.parse.missing_precursor_mz,
            |completed, total| on_progress(JobProgressStage::LoadingLibrary, completed, total),
        )?;
        let prepared = prepare_loaded_library(loaded, request.search.compute)?;
        *cached = Some(CachedPreparedLibrary {
            path: canonical,
            fingerprint,
            identifier: identifier.to_string(),
            min_peaks: request.parse.min_peaks,
            max_peaks: request.parse.max_peaks,
            missing_precursor_policy: request.parse.missing_precursor_mz,
            compute: request.search.compute,
            precursor_mz_tolerance: request.search.precursor_mz_tolerance,
            prepared: Arc::clone(&prepared),
        });
        return Ok(prepared);
    }

    let loaded = load_request_spectra_with_progress(
        JobProgressStage::LoadingLibrary,
        &request.library_source_label,
        request.library_mgf_text.as_deref(),
        None,
        identifier,
        request.parse.min_peaks,
        request.parse.max_peaks,
        request.parse.missing_precursor_mz,
        on_progress,
    )?;
    prepare_loaded_library(Arc::new(loaded), request.search.compute)
}

#[cfg(not(target_arch = "wasm32"))]
fn prepare_loaded_library(
    loaded: Arc<LoadedSpectra>,
    compute: ComputeParams,
) -> Result<Arc<PreparedLibrary>, String> {
    let processed = preprocess_spectra_for_metric(loaded.spectra.clone(), compute)?;
    Ok(Arc::new(PreparedLibrary {
        loaded,
        processed,
        index: OnceLock::new(),
        search_states: OnceLock::new(),
    }))
}

/// Loads request spectra with stage-aware progress reporting in native builds.
#[cfg(not(target_arch = "wasm32"))]
fn load_request_spectra_with_progress<F>(
    stage: JobProgressStage,
    source_label: &str,
    mgf_text: Option<&str>,
    mgf_path: Option<&str>,
    identifier: &str,
    min_peaks: usize,
    max_peaks: usize,
    missing_precursor_policy: MissingPrecursorMz,
    on_progress: &F,
) -> Result<LoadedSpectra, String>
where
    F: Fn(JobProgressStage, u64, u64) + Sync,
{
    if let Some(path) = mgf_path {
        return load_mgf_path_cached_with_progress(
            Path::new(path),
            identifier,
            min_peaks,
            max_peaks,
            missing_precursor_policy,
            |completed, total| on_progress(stage, completed, total),
        )
        .map(|loaded| loaded.as_ref().clone());
    }

    if let Some(text) = mgf_text {
        let total = text.len() as u64;
        on_progress(stage, 0, total.max(1));
        let loaded = load_mgf_bytes_with_policy(
            source_label,
            text.as_bytes(),
            identifier,
            min_peaks,
            max_peaks,
            missing_precursor_policy,
        )?;
        on_progress(stage, total.max(1), total.max(1));
        return Ok(loaded);
    }

    Err("request did not provide an MGF source".to_string())
}

/// Loads an MGF file through the native cache while streaming byte progress to the caller.
#[cfg(not(target_arch = "wasm32"))]
fn load_mgf_path_cached_with_progress<F>(
    path: &Path,
    identifier: &str,
    min_peaks: usize,
    max_peaks: usize,
    missing_precursor_policy: MissingPrecursorMz,
    mut on_progress: F,
) -> Result<Arc<LoadedSpectra>, String>
where
    F: FnMut(u64, u64),
{
    let canonical = std::fs::canonicalize(path)
        .map_err(|err| format!("cannot resolve {}: {err}", path.display()))?;
    let fingerprint = file_fingerprint(&canonical)?;
    let total = fingerprint.len.max(1);

    if let Ok(cache) = mgf_cache().lock()
        && let Some(cached) = cache.get(&canonical)
        && cached.fingerprint == fingerprint
        && cached.identifier == identifier
        && cached.min_peaks == min_peaks
        && cached.max_peaks == max_peaks
        && cached.missing_precursor_policy == missing_precursor_policy
    {
        on_progress(total, total);
        return Ok(cached.loaded.clone());
    }

    let handle = start_native_mgf_load_with_policy(
        &canonical,
        identifier,
        min_peaks,
        max_peaks,
        missing_precursor_policy,
    )?;
    on_progress(0, total);
    loop {
        let processed = handle.processed_bytes().min(handle.total_bytes()).max(0);
        on_progress(processed, total);
        if let Some(message) = handle.try_recv() {
            match message {
                NativeLoadMessage::Finished(loaded) => {
                    let loaded = Arc::new(loaded);
                    on_progress(total, total);
                    if let Ok(mut cache) = mgf_cache().lock() {
                        cache.insert(
                            canonical,
                            CachedMgf {
                                fingerprint,
                                identifier: identifier.to_string(),
                                min_peaks,
                                max_peaks,
                                missing_precursor_policy,
                                loaded: Arc::clone(&loaded),
                            },
                        );
                    }
                    return Ok(loaded);
                }
                NativeLoadMessage::Failed(err) => return Err(err),
            }
        }
        std::thread::sleep(Duration::from_millis(16));
    }
}

/// Computes a lightweight file fingerprint used by the native MGF cache.
#[cfg(not(target_arch = "wasm32"))]
fn file_fingerprint(path: &Path) -> Result<FileFingerprint, String> {
    let metadata =
        std::fs::metadata(path).map_err(|err| format!("cannot stat {}: {err}", path.display()))?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| (duration.as_secs(), duration.subsec_nanos()));
    Ok(FileFingerprint {
        len: metadata.len(),
        modified,
    })
}

/// Starts a background native search worker over already-loaded query and library spectra.
pub fn start_native_search(
    queries: Vec<SpectrumRecord>,
    library: Vec<SpectrumRecord>,
    params: LibrarySearchParams,
) -> NativeSearchHandle {
    let total = total_search_pairs(queries.len(), library.len());
    let done = Arc::new(AtomicUsize::new(0));
    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    #[cfg(not(target_arch = "wasm32"))]
    let done_for_thread = Arc::clone(&done);
    #[cfg(not(target_arch = "wasm32"))]
    let cancel_for_thread = Arc::clone(&cancel);

    std::thread::spawn(move || {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let query_count = queries.len();
            let library_count = library.len();

            let queries = match preprocess_spectra_for_metric(queries, params.compute) {
                Ok(spectra) => spectra,
                Err(err) => {
                    let _ = tx.send(SearchMessage::Failed(err));
                    return;
                }
            };
            let library = match preprocess_spectra_for_metric(library, params.compute) {
                Ok(spectra) => spectra,
                Err(err) => {
                    let _ = tx.send(SearchMessage::Failed(err));
                    return;
                }
            };
            let total = total_search_pairs(query_count, library_count);
            let hits = if let Some(index) = match MetricIndex::new(params.compute, &library) {
                Some(Ok(index)) => Some(index),
                Some(Err(err)) => {
                    let _ = tx.send(SearchMessage::Failed(err));
                    return;
                }
                None => None,
            } {
                let scratch = IndexedSearchScratch::new(&index);
                score_query_library_pairs_indexed(
                    &queries,
                    &library,
                    &params,
                    &index,
                    &scratch,
                    total,
                    done_for_thread,
                    cancel_for_thread,
                    &|_, _, _| {},
                    &|| false,
                )
            } else {
                let scorer = match MetricScorer::new(params.compute) {
                    Ok(sim) => sim,
                    Err(err) => {
                        let _ = tx.send(SearchMessage::Failed(err));
                        return;
                    }
                };
                score_query_library_pairs(
                    &queries,
                    &library,
                    &params,
                    &scorer,
                    total,
                    done_for_thread,
                    cancel_for_thread,
                    &|_, _, _| {},
                    &|| false,
                )
            };

            let result = match hits {
                Ok(Some(hits)) => SearchResult {
                    hits,
                    query_count,
                    library_count,
                    metric: params.compute.metric,
                },
                Ok(None) => {
                    let _ = tx.send(SearchMessage::Cancelled);
                    return;
                }
                Err(err) => {
                    let _ = tx.send(SearchMessage::Failed(err));
                    return;
                }
            };

            let _ = tx.send(SearchMessage::Finished(result));
        }

        #[cfg(target_arch = "wasm32")]
        {
            let _ = (queries, library, params);
            let _ = tx.send(SearchMessage::Failed(
                "native threaded search is unavailable on wasm".to_string(),
            ));
        }
    });

    NativeSearchHandle {
        total,
        done,
        cancel,
        rx,
    }
}

pub enum IncrementalSearchStep {
    Progress,
    Finished(SearchResult),
    Cancelled,
}

pub struct IncrementalSearchState {
    params: LibrarySearchParams,
    scorer: MetricScorer,
    queries: Vec<SpectrumRecord>,
    library: Vec<SpectrumRecord>,
    query_index: usize,
    library_index: usize,
    total: usize,
    done: usize,
    cancel: bool,
    candidates: Vec<SearchCandidate>,
}

impl IncrementalSearchState {
    pub fn new(
        queries: Vec<SpectrumRecord>,
        library: Vec<SpectrumRecord>,
        params: LibrarySearchParams,
    ) -> Result<Self, String> {
        let queries = preprocess_spectra_for_metric(queries, params.compute)?;
        let library = preprocess_spectra_for_metric(library, params.compute)?;
        let total = total_search_pairs(queries.len(), library.len());
        let scorer = MetricScorer::new(params.compute)?;
        Ok(Self {
            params,
            scorer,
            queries,
            library,
            query_index: 0,
            library_index: 0,
            total,
            done: 0,
            cancel: false,
            candidates: Vec::new(),
        })
    }

    pub fn total(&self) -> usize {
        self.total
    }

    pub fn done(&self) -> usize {
        self.done
    }

    pub fn cancel(&mut self) {
        self.cancel = true;
    }

    pub fn step(&mut self, budget: usize) -> Result<IncrementalSearchStep, String> {
        if self.cancel {
            return Ok(IncrementalSearchStep::Cancelled);
        }

        let query_count = self.queries.len();
        let library_count = self.library.len();
        if query_count == 0 || library_count == 0 || self.query_index >= query_count {
            return Ok(IncrementalSearchStep::Finished(SearchResult {
                hits: finalize_search_candidates(
                    std::mem::take(&mut self.candidates),
                    self.params.top_n,
                ),
                query_count,
                library_count,
                metric: self.params.compute.metric,
            }));
        }

        let mut remaining = budget.max(1);
        while remaining > 0 && self.query_index < query_count {
            if self.cancel {
                return Ok(IncrementalSearchStep::Cancelled);
            }

            if search_precursor_mz_passes(
                &self.queries[self.query_index],
                &self.library[self.library_index],
                &self.params,
            ) {
                let left = self.queries[self.query_index].spectrum.as_ref();
                let right = self.library[self.library_index].spectrum.as_ref();
                let (score, matches) =
                    self.scorer
                        .similarity(left, right, self.query_index, self.library_index)?;
                if search_match_passes(score, matches, &self.params) {
                    self.candidates.push(SearchCandidate {
                        query_index: self.query_index,
                        library_index: self.library_index,
                        rank_before_taxonomy: 0,
                        spectral_score: score,
                        taxonomic_score: 0.0,
                        combined_score: score,
                        matches,
                        matched_organism_name: None,
                        matched_organism_wikidata: None,
                        matched_shared_rank: None,
                        matched_short_inchikey: None,
                    });
                }
            }
            self.done += 1;

            self.library_index += 1;
            if self.library_index >= library_count {
                self.query_index += 1;
                self.library_index = 0;
            }
            remaining -= 1;
        }

        if self.query_index >= query_count {
            Ok(IncrementalSearchStep::Finished(SearchResult {
                hits: finalize_search_candidates(
                    std::mem::take(&mut self.candidates),
                    self.params.top_n,
                ),
                query_count,
                library_count,
                metric: self.params.compute.metric,
            }))
        } else {
            Ok(IncrementalSearchStep::Progress)
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn search_candidates(
    queries: &[SpectrumRecord],
    library: &[SpectrumRecord],
    params: &LibrarySearchParams,
    scorer: &MetricScorer,
) -> Result<Vec<SearchCandidate>, String> {
    let mut candidates = Vec::new();
    for query_idx in 0..queries.len() {
        for library_idx in 0..library.len() {
            if !search_precursor_mz_passes(&queries[query_idx], &library[library_idx], params) {
                continue;
            }
            let left = queries[query_idx].spectrum.as_ref();
            let right = library[library_idx].spectrum.as_ref();
            let (score, matches) = scorer.similarity(left, right, query_idx, library_idx)?;
            if search_match_passes(score, matches, params) {
                candidates.push(SearchCandidate {
                    query_index: query_idx,
                    library_index: library_idx,
                    rank_before_taxonomy: 0,
                    spectral_score: score,
                    taxonomic_score: 0.0,
                    combined_score: score,
                    matches,
                    matched_organism_name: None,
                    matched_organism_wikidata: None,
                    matched_shared_rank: None,
                    matched_short_inchikey: None,
                });
            }
        }
    }
    Ok(candidates)
}

/// Scores only precursor-compatible query/library pairs in parallel.
#[cfg(not(target_arch = "wasm32"))]
fn score_query_library_pairs(
    queries: &[SpectrumRecord],
    library: &[SpectrumRecord],
    params: &LibrarySearchParams,
    scorer: &MetricScorer,
    total: usize,
    done_worker: Arc<AtomicUsize>,
    cancel_worker: Arc<AtomicBool>,
    on_progress: &(impl Fn(JobProgressStage, u64, u64) + Sync),
    is_cancelled: &(impl Fn() -> bool + Sync),
) -> Result<Option<Vec<CandidateHit>>, String> {
    if library.is_empty() {
        return Ok(Some(Vec::new()));
    }

    // A sorted precursor lookup avoids visiting the full query × library product.
    // Keep original indices so score ties and exported references remain unchanged.
    let mut precursors: Vec<(f64, usize)> = library
        .iter()
        .enumerate()
        .map(|(index, record)| (record.meta.precursor_mz, index))
        .collect();
    precursors
        .sort_unstable_by(|left, right| left.0.total_cmp(&right.0).then(left.1.cmp(&right.1)));
    let finite_start =
        precursors.partition_point(|(mz, _)| mz.total_cmp(&f64::NEG_INFINITY).is_le());
    let finite_end = precursors.partition_point(|(mz, _)| mz.total_cmp(&f64::INFINITY).is_lt());
    let finite = &precursors[finite_start..finite_end];

    let error = Arc::new(Mutex::new(None::<String>));
    let error_worker = Arc::clone(&error);
    let done_for_iter = Arc::clone(&done_worker);
    let cancel_for_iter = Arc::clone(&cancel_worker);
    let report_every = progress_report_every(total);

    let candidates: Vec<SearchCandidate> = queries
        .par_iter()
        .enumerate()
        .flat_map_iter(|(query_idx, query)| {
            let was_cancelled = cancel_for_iter.load(Ordering::Relaxed) || is_cancelled();
            if was_cancelled {
                cancel_for_iter.store(true, Ordering::Relaxed);
            }

            let mz = query.meta.precursor_mz;
            let tolerance = params.precursor_mz_tolerance;
            let compatible = if was_cancelled {
                &precursors[..0]
            } else if mz.is_finite() && tolerance.is_finite() && tolerance >= 0.0 {
                // Comparing the actual subtraction (not mz ± tolerance) retains
                // the original floating-point boundary behavior exactly.
                let start = finite.partition_point(|(other, _)| mz - other > tolerance);
                let end = finite.partition_point(|(other, _)| other - mz <= tolerance);
                &finite[start..end]
            } else if tolerance.is_sign_positive() && tolerance.is_infinite() && !mz.is_nan() {
                &precursors[..]
            } else {
                &precursors[..0]
            };

            if !was_cancelled {
                let completed =
                    done_for_iter.fetch_add(library.len(), Ordering::Relaxed) + library.len();
                if completed / report_every != (completed - library.len()) / report_every
                    || completed >= total
                {
                    on_progress(
                        JobProgressStage::Scoring,
                        completed.min(total) as u64,
                        total as u64,
                    );
                }
            }

            let cancel = &cancel_for_iter;
            let error_slot = &error_worker;
            compatible.iter().filter_map(move |&(_, library_idx)| {
                if cancel.load(Ordering::Relaxed) || is_cancelled() {
                    cancel.store(true, Ordering::Relaxed);
                    return None;
                }
                if !search_precursor_mz_passes(query, &library[library_idx], params) {
                    return None;
                }
                let left = query.spectrum.as_ref();
                let right = library[library_idx].spectrum.as_ref();
                match scorer.similarity(left, right, query_idx, library_idx) {
                    Ok((score, matches)) => {
                        search_match_passes(score, matches, params).then_some(SearchCandidate {
                            query_index: query_idx,
                            library_index: library_idx,
                            rank_before_taxonomy: 0,
                            spectral_score: score,
                            taxonomic_score: 0.0,
                            combined_score: score,
                            matches,
                            matched_organism_name: None,
                            matched_organism_wikidata: None,
                            matched_shared_rank: None,
                            matched_short_inchikey: None,
                        })
                    }
                    Err(err) => {
                        if let Ok(mut slot) = error_slot.lock()
                            && slot.is_none()
                        {
                            *slot = Some(err);
                        }
                        cancel.store(true, Ordering::Relaxed);
                        None
                    }
                }
            })
        })
        .collect();

    if let Ok(mut slot) = error.lock()
        && let Some(err) = slot.take()
    {
        return Err(err);
    }
    if cancel_worker.load(Ordering::Relaxed) {
        return Ok(None);
    }
    Ok(Some(finalize_search_candidates(candidates, params.top_n)))
}

/// Scores query/library matches with a reusable inverted spectral index.
#[cfg(not(target_arch = "wasm32"))]
fn score_query_library_pairs_indexed(
    queries: &[SpectrumRecord],
    library: &[SpectrumRecord],
    params: &LibrarySearchParams,
    index: &MetricIndex,
    scratch: &IndexedSearchScratch,
    total: usize,
    done_worker: Arc<AtomicUsize>,
    cancel_worker: Arc<AtomicBool>,
    on_progress: &(impl Fn(JobProgressStage, u64, u64) + Sync),
    is_cancelled: &(impl Fn() -> bool + Sync),
) -> Result<Option<Vec<CandidateHit>>, String> {
    let error = Arc::new(Mutex::new(None::<String>));
    let error_worker = Arc::clone(&error);
    let done_for_iter = Arc::clone(&done_worker);
    let cancel_for_iter = Arc::clone(&cancel_worker);
    let report_every = progress_report_every(total);

    let candidates: Vec<SearchCandidate> = queries
        .par_iter()
        .enumerate()
        .map_init(
            || scratch.for_current_worker(),
            |state, (query_idx, query)| {
                if cancel_for_iter.load(Ordering::Relaxed) || is_cancelled() {
                    cancel_for_iter.store(true, Ordering::Relaxed);
                    return Vec::new();
                }

                let results = match {
                    let mut state = state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    index.search_with_state(query, &mut state, query_idx)
                } {
                    Ok(results) => results,
                    Err(err) => {
                        if let Ok(mut slot) = error_worker.lock()
                            && slot.is_none()
                        {
                            *slot = Some(err);
                        }
                        cancel_for_iter.store(true, Ordering::Relaxed);
                        return Vec::new();
                    }
                };

                let completed =
                    done_for_iter.fetch_add(library.len(), Ordering::Relaxed) + library.len();
                if completed % report_every == 0 || completed >= total {
                    on_progress(
                        JobProgressStage::Scoring,
                        completed.min(total) as u64,
                        total as u64,
                    );
                }

                results
                    .into_iter()
                    .filter_map(|hit| {
                        let library_idx = hit.library_index;
                        if library_idx >= library.len() {
                            return None;
                        }
                        if !search_precursor_mz_passes(query, &library[library_idx], params) {
                            return None;
                        }
                        search_match_passes(hit.score, hit.matches, params).then_some(
                            SearchCandidate {
                                query_index: query_idx,
                                library_index: library_idx,
                                rank_before_taxonomy: 0,
                                spectral_score: hit.score,
                                taxonomic_score: 0.0,
                                combined_score: hit.score,
                                matches: hit.matches,
                                matched_organism_name: None,
                                matched_organism_wikidata: None,
                                matched_shared_rank: None,
                                matched_short_inchikey: None,
                            },
                        )
                    })
                    .collect::<Vec<_>>()
            },
        )
        .flat_map_iter(|hits| hits)
        .collect();

    if let Ok(mut slot) = error.lock()
        && let Some(err) = slot.take()
    {
        return Err(err);
    }
    if cancel_worker.load(Ordering::Relaxed) {
        return Ok(None);
    }
    Ok(Some(finalize_search_candidates(candidates, params.top_n)))
}

/// Scores all unique spectrum pairs serially.
#[cfg(target_arch = "wasm32")]
fn score_all_pairs(
    spectra: &[SpectrumRecord],
    scorer: &MetricScorer,
    min_matched_peaks: usize,
) -> Result<Vec<PairScore>, String> {
    let mut pairs = Vec::with_capacity(total_pairs(spectra.len()));
    for left_index in 0..spectra.len() {
        for right_index in left_index..spectra.len() {
            let left = spectra[left_index].spectrum.as_ref();
            let right = spectra[right_index].spectrum.as_ref();
            let (score, matches) = scorer.similarity(left, right, left_index, right_index)?;
            if matches < min_matched_peaks {
                continue;
            }
            pairs.push(PairScore {
                left: left_index,
                right: right_index,
                score,
                matches,
            });
        }
    }
    Ok(pairs)
}

/// Scores network neighbors in parallel while maintaining a bounded neighbor heap per node.
#[cfg(not(target_arch = "wasm32"))]
fn score_network_neighbors_parallel(
    spectra: &[SpectrumRecord],
    scorer: &MetricScorer,
    threshold: f64,
    min_matched_peaks: usize,
    top_k: usize,
    total: usize,
    done_worker: Arc<AtomicUsize>,
    on_progress: &(impl Fn(JobProgressStage, u64, u64) + Sync),
    is_cancelled: &(impl Fn() -> bool + Sync),
) -> Result<Vec<Vec<SelectedNeighbor>>, String> {
    let threshold = threshold.clamp(0.0, 1.0);
    let top_k = top_k.max(1);
    let report_every = progress_report_every(total);
    let neighbors: Vec<Mutex<BinaryHeap<Reverse<RankedNeighbor>>>> = (0..spectra.len())
        .map(|_| Mutex::new(BinaryHeap::new()))
        .collect();
    let error = Arc::new(Mutex::new(None::<String>));
    let error_worker = Arc::clone(&error);

    (0..spectra.len()).into_par_iter().for_each(|left_index| {
        if is_cancelled() {
            if let Ok(mut slot) = error_worker.lock()
                && slot.is_none()
            {
                *slot = Some("job cancelled".to_string());
            }
            return;
        }
        let mut local_heap: BinaryHeap<Reverse<RankedNeighbor>> = BinaryHeap::new();
        let left = spectra[left_index].spectrum.as_ref();

        for right_index in (left_index + 1)..spectra.len() {
            if is_cancelled() {
                if let Ok(mut slot) = error_worker.lock()
                    && slot.is_none()
                {
                    *slot = Some("job cancelled".to_string());
                }
                return;
            }
            let completed = done_worker.fetch_add(1, Ordering::Relaxed) + 1;
            if completed % report_every == 0 || completed == total {
                on_progress(JobProgressStage::Scoring, completed as u64, total as u64);
            }
            let right = spectra[right_index].spectrum.as_ref();
            match scorer.similarity(left, right, left_index, right_index) {
                Ok((score, matches)) => {
                    if score < threshold {
                        continue;
                    }
                    if matches < min_matched_peaks {
                        continue;
                    }
                    let left_candidate = RankedNeighbor {
                        neighbor: right_index,
                        score,
                        matches,
                    };
                    let right_candidate = RankedNeighbor {
                        neighbor: left_index,
                        score,
                        matches,
                    };
                    push_ranked_neighbor(&mut local_heap, left_candidate, top_k);
                    if let Ok(mut heap) = neighbors[right_index].lock() {
                        push_ranked_neighbor(&mut heap, right_candidate, top_k);
                    }
                }
                Err(err) => {
                    if let Ok(mut slot) = error_worker.lock()
                        && slot.is_none()
                    {
                        *slot = Some(err);
                    }
                }
            }
        }

        if let Ok(mut heap) = neighbors[left_index].lock() {
            for ranked in local_heap.into_iter().map(|entry| entry.0) {
                push_ranked_neighbor(&mut heap, ranked, top_k);
            }
        }
    });

    if let Ok(mut slot) = error.lock()
        && let Some(err) = slot.take()
    {
        return Err(err);
    }

    Ok(neighbors
        .into_iter()
        .map(|heap| ranked_heap_to_selected_neighbors(heap, top_k))
        .collect())
}

/// Scores network neighbors with a reusable inverted spectral index.
#[cfg(not(target_arch = "wasm32"))]
fn score_network_neighbors_indexed_parallel(
    spectra: &[SpectrumRecord],
    index: &MetricIndex,
    threshold: f64,
    min_matched_peaks: usize,
    top_k: usize,
    total: usize,
    done_worker: Arc<AtomicUsize>,
    on_progress: &(impl Fn(JobProgressStage, u64, u64) + Sync),
    is_cancelled: &(impl Fn() -> bool + Sync),
) -> Result<Vec<Vec<SelectedNeighbor>>, String> {
    let threshold = threshold.clamp(0.0, 1.0);
    let top_k = top_k.max(1);
    let report_every = progress_report_every(total);
    let neighbors: Vec<Mutex<BinaryHeap<Reverse<RankedNeighbor>>>> = (0..spectra.len())
        .map(|_| Mutex::new(BinaryHeap::new()))
        .collect();
    let error = Arc::new(Mutex::new(None::<String>));
    let error_worker = Arc::clone(&error);

    (0..spectra.len()).into_par_iter().for_each(|left_index| {
        if is_cancelled() {
            if let Ok(mut slot) = error_worker.lock()
                && slot.is_none()
            {
                *slot = Some("job cancelled".to_string());
            }
            return;
        }

        let mut state = index.new_search_state();
        let results = match index.search_with_state(&spectra[left_index], &mut state, left_index) {
            Ok(results) => results,
            Err(err) => {
                if let Ok(mut slot) = error_worker.lock()
                    && slot.is_none()
                {
                    *slot = Some(err);
                }
                return;
            }
        };

        let row_pairs = spectra.len().saturating_sub(left_index + 1);
        let completed = done_worker.fetch_add(row_pairs, Ordering::Relaxed) + row_pairs;
        if completed % report_every == 0 || completed >= total {
            on_progress(
                JobProgressStage::Scoring,
                completed.min(total) as u64,
                total as u64,
            );
        }

        let mut local_heap: BinaryHeap<Reverse<RankedNeighbor>> = BinaryHeap::new();
        for hit in results {
            let right_index = hit.library_index;
            if right_index <= left_index || right_index >= spectra.len() {
                continue;
            }
            if hit.score < threshold || hit.matches < min_matched_peaks {
                continue;
            }

            let left_candidate = RankedNeighbor {
                neighbor: right_index,
                score: hit.score,
                matches: hit.matches,
            };
            let right_candidate = RankedNeighbor {
                neighbor: left_index,
                score: hit.score,
                matches: hit.matches,
            };
            push_ranked_neighbor(&mut local_heap, left_candidate, top_k);
            if let Ok(mut heap) = neighbors[right_index].lock() {
                push_ranked_neighbor(&mut heap, right_candidate, top_k);
            }
        }

        if let Ok(mut heap) = neighbors[left_index].lock() {
            for ranked in local_heap.into_iter().map(|entry| entry.0) {
                push_ranked_neighbor(&mut heap, ranked, top_k);
            }
        }
    });

    if let Ok(mut slot) = error.lock()
        && let Some(err) = slot.take()
    {
        return Err(err);
    }

    Ok(neighbors
        .into_iter()
        .map(|heap| ranked_heap_to_selected_neighbors(heap, top_k))
        .collect())
}

/// Pushes a candidate into a bounded max-heap used for top-k neighbor retention.
#[cfg(not(target_arch = "wasm32"))]
fn push_ranked_neighbor(
    heap: &mut BinaryHeap<Reverse<RankedNeighbor>>,
    candidate: RankedNeighbor,
    top_k: usize,
) {
    if heap.len() < top_k {
        heap.push(Reverse(candidate));
        return;
    }

    if heap.peek().is_some_and(|worst| candidate > worst.0) {
        let _ = heap.pop();
        heap.push(Reverse(candidate));
    }
}

/// Chooses how often long-running scoring loops emit progress updates.
#[cfg(not(target_arch = "wasm32"))]
fn progress_report_every(total: usize) -> usize {
    (total / 1000).max(10_000)
}

/// Converts a bounded ranking heap into a sorted selected-neighbor list.
#[cfg(not(target_arch = "wasm32"))]
fn ranked_heap_to_selected_neighbors(
    heap: Mutex<BinaryHeap<Reverse<RankedNeighbor>>>,
    top_k: usize,
) -> Vec<SelectedNeighbor> {
    let Ok(heap) = heap.into_inner() else {
        return Vec::new();
    };
    let mut ranked: Vec<RankedNeighbor> = heap.into_iter().map(|entry| entry.0).collect();
    ranked.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(a.neighbor.cmp(&b.neighbor))
    });
    ranked.truncate(top_k);
    ranked
        .into_iter()
        .map(|neighbor| SelectedNeighbor {
            neighbor: neighbor.neighbor,
            score: neighbor.score,
            matches: neighbor.matches,
        })
        .collect()
}

/// Returns whether a scored candidate satisfies match-count and score thresholds.
fn search_match_passes(score: f64, matches: usize, params: &LibrarySearchParams) -> bool {
    matches >= params.min_matched_peaks && score >= params.min_similarity_threshold
}

/// Returns whether a library candidate satisfies precursor m/z prefiltering.
fn search_precursor_mz_passes(
    query: &SpectrumRecord,
    library: &SpectrumRecord,
    params: &LibrarySearchParams,
) -> bool {
    (query.meta.precursor_mz - library.meta.precursor_mz).abs() <= params.precursor_mz_tolerance
}

/// Finalizes ranked candidate hits, keeping only the best `top_n` per query.
fn finalize_search_candidates(candidates: Vec<SearchCandidate>, top_n: usize) -> Vec<CandidateHit> {
    let mut by_query: Vec<Vec<SearchCandidate>> = Vec::new();
    for candidate in candidates {
        if by_query.len() <= candidate.query_index {
            by_query.resize_with(candidate.query_index + 1, Vec::new);
        }
        by_query[candidate.query_index].push(candidate);
    }

    let keep = top_n.max(1);
    let mut hits = Vec::new();
    for (query_index, mut query_hits) in by_query.into_iter().enumerate() {
        query_hits.sort_by(search_candidate_order);
        for (rank, hit) in query_hits.into_iter().take(keep).enumerate() {
            hits.push(CandidateHit {
                query_index,
                library_index: hit.library_index,
                rank: rank + 1,
                spectral_score: hit.spectral_score,
                matches: hit.matches,
                payload: (),
            });
        }
    }
    hits.sort_by(|left, right| {
        left.query_index
            .cmp(&right.query_index)
            .then(left.rank.cmp(&right.rank))
    });
    hits
}

/// Builds the enriched search artifact result, optionally applying taxonomic reranking.
fn build_search_artifact_result(
    base: SearchResult,
    queries: &[SpectrumRecord],
    library: &[SpectrumRecord],
    taxonomy: Option<&SearchTaxonomyConfig>,
    top_n: usize,
) -> SearchArtifactResult {
    let hits = enrich_search_hits(base.hits, queries, library, taxonomy, top_n);
    SearchArtifactResult {
        hits,
        query_count: base.query_count,
        library_count: base.library_count,
        metric: base.metric,
        taxonomic_reranking_applied: taxonomy.is_some(),
        taxonomic_query: taxonomy.map(|config| config.query.query_label.clone()),
    }
}

/// Enriches base search hits with deviation and optional taxonomy metadata.
fn enrich_search_hits(
    base_hits: Vec<CandidateHit>,
    queries: &[SpectrumRecord],
    library: &[SpectrumRecord],
    taxonomy: Option<&SearchTaxonomyConfig>,
    top_n: usize,
) -> Vec<SearchArtifactHit> {
    if let Some(taxonomy) = taxonomy {
        let candidates = base_hits
            .iter()
            .map(|hit| build_search_candidate(hit, library, Some(taxonomy)))
            .collect::<Vec<_>>();
        finalize_search_artifact_candidates(candidates, queries, library, top_n)
    } else {
        base_hits
            .into_iter()
            .map(|hit| {
                let query = &queries[hit.query_index];
                let library_hit = &library[hit.library_index];
                SearchArtifactHit {
                    query_index: hit.query_index,
                    library_index: hit.library_index,
                    rank: hit.rank,
                    rank_before_taxonomy: None,
                    spectral_score: hit.spectral_score,
                    ms1_deviation_ppm: ms1_deviation_ppm(
                        query.meta.precursor_mz,
                        library_hit.meta.precursor_mz,
                    ),
                    taxonomic_score: 0.0,
                    combined_score: hit.spectral_score,
                    matches: hit.matches,
                    matched_organism_name: None,
                    matched_organism_wikidata: None,
                    matched_shared_rank: None,
                    matched_short_inchikey: None,
                }
            })
            .collect()
    }
}

/// Combines a base spectral hit with optional taxonomic metadata.
fn build_search_candidate(
    hit: &CandidateHit,
    library: &[SpectrumRecord],
    taxonomy: Option<&SearchTaxonomyConfig>,
) -> SearchCandidate {
    let library_record = &library[hit.library_index];
    let (
        taxonomic_score,
        matched_organism_name,
        matched_organism_wikidata,
        matched_shared_rank,
        matched_short_inchikey,
    ) = if let Some(config) = taxonomy {
        let short_inchikey = short_inchikey_from_record(library_record);
        match short_inchikey
            .as_deref()
            .and_then(|short| config.lotus.match_candidate(short, &config.query.lineage))
        {
            Some(matched) => (
                f64::from(matched.score),
                matched.matched_organism_name,
                matched.matched_organism_wikidata,
                matched.shared_rank.map(|rank| rank.label().to_string()),
                matched.matched_short_inchikey,
            ),
            None => (0.0, None, None, None, short_inchikey),
        }
    } else {
        (0.0, None, None, None, None)
    };

    SearchCandidate {
        query_index: hit.query_index,
        library_index: hit.library_index,
        rank_before_taxonomy: hit.rank,
        spectral_score: hit.spectral_score,
        taxonomic_score,
        combined_score: hit.spectral_score + taxonomic_score,
        matches: hit.matches,
        matched_organism_name,
        matched_organism_wikidata,
        matched_shared_rank,
        matched_short_inchikey,
    }
}

/// Finalizes enriched search candidates into exported hit rows.
fn finalize_search_artifact_candidates(
    candidates: Vec<SearchCandidate>,
    queries: &[SpectrumRecord],
    library: &[SpectrumRecord],
    top_n: usize,
) -> Vec<SearchArtifactHit> {
    let mut by_query: Vec<Vec<SearchCandidate>> = Vec::new();
    for candidate in candidates {
        if by_query.len() <= candidate.query_index {
            by_query.resize_with(candidate.query_index + 1, Vec::new);
        }
        by_query[candidate.query_index].push(candidate);
    }

    let keep = top_n.max(1);
    let mut hits = Vec::new();
    for (query_index, mut query_hits) in by_query.into_iter().enumerate() {
        query_hits.sort_by(search_candidate_order);
        for (rank, hit) in query_hits.into_iter().take(keep).enumerate() {
            let query = &queries[query_index];
            let library_hit = &library[hit.library_index];
            hits.push(SearchArtifactHit {
                query_index,
                library_index: hit.library_index,
                rank: rank + 1,
                rank_before_taxonomy: Some(hit.rank_before_taxonomy),
                spectral_score: hit.spectral_score,
                ms1_deviation_ppm: ms1_deviation_ppm(
                    query.meta.precursor_mz,
                    library_hit.meta.precursor_mz,
                ),
                taxonomic_score: hit.taxonomic_score,
                combined_score: hit.combined_score,
                matches: hit.matches,
                matched_organism_name: hit.matched_organism_name,
                matched_organism_wikidata: hit.matched_organism_wikidata,
                matched_shared_rank: hit.matched_shared_rank,
                matched_short_inchikey: hit.matched_short_inchikey,
            });
        }
    }
    hits.sort_by(|left, right| {
        left.query_index
            .cmp(&right.query_index)
            .then(left.rank.cmp(&right.rank))
    });
    hits
}

/// Computes precursor deviation in parts per million using query m/z as the denominator.
fn ms1_deviation_ppm(query_precursor_mz: f64, candidate_precursor_mz: f64) -> f64 {
    if !query_precursor_mz.is_finite() || query_precursor_mz == 0.0 {
        return 0.0;
    }
    ((candidate_precursor_mz - query_precursor_mz) / query_precursor_mz) * 1_000_000.0
}

/// Ordering used for taxonomically reranked candidates.
fn search_candidate_order(left: &SearchCandidate, right: &SearchCandidate) -> std::cmp::Ordering {
    right
        .combined_score
        .total_cmp(&left.combined_score)
        .then(right.spectral_score.total_cmp(&left.spectral_score))
        .then(right.matches.cmp(&left.matches))
        .then(left.library_index.cmp(&right.library_index))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;
    #[cfg(not(target_arch = "wasm32"))]
    use std::sync::atomic::AtomicUsize;

    use mass_spectrometry::prelude::{GenericSpectrum, SpectrumAlloc, SpectrumMut};

    #[cfg(not(target_arch = "wasm32"))]
    use super::load_search_taxonomy_config;
    use super::{
        ComputeParams, IncrementalSearchState, IncrementalSearchStep, LibrarySearchParams,
        SearchResult, finalize_search_candidates, run_search_request, search_library,
        start_native_search, total_search_pairs,
    };
    use crate::api::MissingPrecursorMz;
    #[cfg(not(target_arch = "wasm32"))]
    use crate::api::SearchTaxonomyRequest;
    use crate::export::SearchQueryKey;
    use crate::model::{SpectrumMetadata, SpectrumRecord};
    use crate::similarity::{MetricIndex, MetricScorer, SimilarityMetric};

    fn spectrum(id: usize, precursor: f64, peaks: &[(f64, f64)]) -> SpectrumRecord {
        let mut spec = GenericSpectrum::with_capacity(precursor, peaks.len())
            .expect("failed to allocate spectrum");
        for (mz, intensity) in peaks {
            spec.add_peak(*mz, *intensity)
                .expect("failed to add test peak");
        }
        SpectrumRecord {
            meta: SpectrumMetadata {
                id,
                spectrum_id: format!("feature_{id}"),
                label: format!("s{id}"),
                raw_name: format!("s{id}"),
                feature_id: None,
                scans: None,
                filename: None,
                source_scan_usi: None,
                featurelist_feature_id: None,
                headers: BTreeMap::new(),
                precursor_mz: precursor,
                num_peaks: peaks.len(),
            },
            peaks: Arc::new(peaks.to_vec()),
            spectrum: Arc::new(spec),
            payload: (),
        }
    }

    fn base_compute_params(metric: SimilarityMetric) -> ComputeParams {
        ComputeParams {
            metric,
            fragment_mz_tolerance: 0.2,
            mz_power: 0.0,
            intensity_power: 1.0,
            top_n_peaks: None,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn hungarian_search_keeps_precursor_boundaries_and_library_ranking() {
        let peaks = &[(10.0, 1.0), (20.0, 0.8), (30.0, 0.6)];
        let query = spectrum(0, 100.0, peaks);
        let mut nonfinite = spectrum(4, 100.0, peaks);
        nonfinite.meta.precursor_mz = f64::NAN;
        let library = vec![
            spectrum(0, 100.1, peaks),
            spectrum(1, 300.0, peaks),
            spectrum(2, 99.9, peaks),
            spectrum(3, 100.0, peaks),
            nonfinite,
        ];
        let result = search_library(
            vec![query],
            library,
            LibrarySearchParams {
                compute: base_compute_params(SimilarityMetric::HungarianCosine),
                precursor_mz_tolerance: (100.0_f64 - 100.1).abs(),
                min_matched_peaks: 3,
                min_similarity_threshold: 0.1,
                top_n: 2,
            },
        )
        .expect("search succeeds");
        assert_eq!(
            result
                .hits
                .iter()
                .map(|hit| (hit.library_index, hit.rank))
                .collect::<Vec<_>>(),
            vec![(0, 1), (2, 2)]
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn parallel_precursor_lookup_matches_incremental_search() {
        let peaks = &[(10.0, 1.0), (20.0, 0.8), (30.0, 0.6)];
        let queries = vec![
            spectrum(0, 100.0, peaks),
            spectrum(1, 200.0, peaks),
            spectrum(2, 300.0, peaks),
        ];
        let mut nonfinite = spectrum(7, 100.0, peaks);
        nonfinite.meta.precursor_mz = f64::NAN;
        let library = vec![
            spectrum(0, 300.0, peaks),
            spectrum(1, 100.1, peaks),
            spectrum(2, 200.0, peaks),
            spectrum(3, 100.0, peaks),
            spectrum(4, 200.2, peaks),
            spectrum(5, 99.9, peaks),
            spectrum(6, 900.0, peaks),
            nonfinite,
        ];
        for metric in [
            SimilarityMetric::HungarianCosine,
            SimilarityMetric::ModifiedHungarianCosine,
        ] {
            let params = LibrarySearchParams {
                compute: base_compute_params(metric),
                precursor_mz_tolerance: (100.0_f64 - 100.1).abs(),
                min_matched_peaks: 3,
                min_similarity_threshold: 0.1,
                top_n: 2,
            };
            let parallel =
                search_library(queries.clone(), library.clone(), params.clone()).expect("search");
            let mut incremental =
                IncrementalSearchState::new(queries.clone(), library.clone(), params)
                    .expect("incremental search");
            let serial = loop {
                match incremental.step(3).expect("incremental step") {
                    IncrementalSearchStep::Progress => {}
                    IncrementalSearchStep::Finished(result) => break result,
                    IncrementalSearchStep::Cancelled => panic!("unexpected cancellation"),
                }
            };
            assert_eq!(parallel.hits, serial.hits, "{metric:?}");
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn file_based_lotus_index_is_reused_and_invalidated_when_file_changes() {
        let path = std::env::temp_dir().join(format!(
            "spectral-matcher-lotus-cache-{}-{}.csv",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        let header = "structure_inchikey,organism_wikidata,organism_name,organism_taxonomy_01domain,organism_taxonomy_02kingdom,organism_taxonomy_03phylum,organism_taxonomy_04class,organism_taxonomy_05order,organism_taxonomy_06family,organism_taxonomy_07tribe,organism_taxonomy_08genus,organism_taxonomy_09species,organism_taxonomy_10varietas\n";
        let first_row = "ABCDEFGHIJKLMN-AAAA,Q1,Withania somnifera,Eukaryota,Archaeplastida,Streptophyta,Magnoliopsida,Solanales,Solanaceae,NA,Withania,Withania somnifera,NA\n";
        std::fs::write(&path, format!("{header}{first_row}")).expect("write LOTUS CSV");

        let mut request = SearchTaxonomyRequest {
            query_text: "Withania somnifera".to_string(),
            lotus_source_label: "test LOTUS".to_string(),
            lotus_csv_text: None,
            lotus_csv_path: Some(path.to_str().expect("UTF-8 temporary path").to_string()),
        };
        let first = load_search_taxonomy_config(&request).expect("first lookup");
        request.query_text = "Q1".to_string();
        let second = load_search_taxonomy_config(&request).expect("cached lookup");
        assert!(Arc::ptr_eq(&first.lotus, &second.lotus));
        assert_eq!(second.query.query_label, "Q1");

        let second_row = "ZZZZZZZZZZZZZZ-BBBB,Q2,Panax ginseng,Eukaryota,Archaeplastida,Streptophyta,Magnoliopsida,Apiales,Araliaceae,NA,Panax,Panax ginseng,NA\n";
        std::fs::write(&path, format!("{header}{first_row}{second_row}"))
            .expect("update LOTUS CSV");
        request.query_text = "Panax ginseng".to_string();
        let updated = load_search_taxonomy_config(&request).expect("updated lookup");
        assert!(!Arc::ptr_eq(&first.lotus, &updated.lotus));
        assert_eq!(updated.query.query_label, "Panax ginseng");
        std::fs::remove_file(&path).expect("remove temporary LOTUS CSV");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn file_library_search_reflects_changed_library_and_metric_settings() {
        let path = std::env::temp_dir().join(format!(
            "spectral-matcher-library-{}-{}.mgf",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        let matching = "BEGIN IONS\nNAME=matching\nPEPMASS=100\n10 100\n20 80\n30 50\nEND IONS\n";
        std::fs::write(&path, matching).expect("write library");
        let mut request = crate::api::SearchRequest {
            query_source_label: "query.mgf".to_string(),
            query_mgf_text: Some(matching.replace("matching", "query")),
            query_mgf_path: None,
            library_source_label: path.display().to_string(),
            library_mgf_text: None,
            library_mgf_path: Some(path.display().to_string()),
            parse: crate::api::ParseConfig {
                identifier: "NAME".to_string(),
                query_identifier: None,
                library_identifier: None,
                min_peaks: 3,
                max_peaks: 1000,
                missing_precursor_mz: MissingPrecursorMz::Reject,
            },
            search: LibrarySearchParams {
                compute: base_compute_params(SimilarityMetric::ModifiedLinearCosine),
                precursor_mz_tolerance: 0.1,
                min_matched_peaks: 3,
                min_similarity_threshold: 0.1,
                top_n: 10,
            },
            taxonomy: None,
            query_key: None,
        };
        let first = run_search_request(request.clone()).expect("first library search");
        assert_eq!(first.result.hits.len(), 1);
        assert_eq!(
            run_search_request(request.clone())
                .expect("repeat")
                .result
                .hits,
            first.result.hits
        );

        std::fs::write(
            &path,
            format!(
                "{matching}{}",
                matching
                    .replace("matching", "second")
                    .replace("PEPMASS=100\n", "PEPMASS=100.05\n")
            ),
        )
        .expect("replace library");
        let updated = run_search_request(request.clone()).expect("updated library search");
        assert_eq!(updated.result.hits.len(), 2);
        assert_eq!(updated.library_stats.accepted, 2);

        request.search.precursor_mz_tolerance = 0.01;
        let narrower = run_search_request(request.clone()).expect("narrow precursor window");
        assert_eq!(narrower.result.hits.len(), 1);
        assert_eq!(narrower.result.hits[0].library_index, 0);
        request.search.precursor_mz_tolerance = 0.1;

        request.search.compute.top_n_peaks = Some(1);
        let truncated = run_search_request(request.clone()).expect("changed peak cap");
        assert!(truncated.result.hits.is_empty());
        request.search.compute.top_n_peaks = None;

        for metric in SimilarityMetric::ALL {
            request.search.compute.metric = metric;
            let result = run_search_request(request.clone()).expect("changed metric");
            assert_eq!(result.result.metric, metric);
            assert_eq!(result.result.hits.len(), 2, "{metric:?}");
        }
        std::fs::remove_file(&path).expect("remove test library");
    }

    #[test]
    fn incremental_search_applies_filters_and_top_n() {
        let queries = vec![
            spectrum(0, 100.0, &[(10.0, 1.0), (20.0, 0.8), (30.0, 0.5)]),
            spectrum(1, 200.0, &[(50.0, 1.0), (60.0, 0.9), (70.0, 0.6)]),
        ];
        let library = vec![
            spectrum(10, 101.0, &[(10.0, 1.0), (20.0, 0.8), (30.0, 0.5)]),
            spectrum(11, 102.0, &[(10.0, 1.0), (20.1, 0.7), (31.0, 0.4)]),
            spectrum(12, 201.0, &[(50.0, 1.0), (60.0, 0.9), (71.0, 0.4)]),
        ];
        let mut state = IncrementalSearchState::new(
            queries,
            library,
            LibrarySearchParams {
                compute: base_compute_params(SimilarityMetric::LinearCosine),
                precursor_mz_tolerance: 5.0,
                min_matched_peaks: 2,
                min_similarity_threshold: 0.1,
                top_n: 1,
            },
        )
        .expect("failed to create search state");

        let result = loop {
            match state.step(2).expect("search step failed") {
                IncrementalSearchStep::Progress => {}
                IncrementalSearchStep::Finished(result) => break result,
                IncrementalSearchStep::Cancelled => panic!("unexpected cancel"),
            }
        };

        assert_eq!(result.hits.len(), 2);
        assert_eq!(result.hits[0].query_index, 0);
        assert_eq!(result.hits[0].library_index, 0);
        assert_eq!(result.hits[1].query_index, 1);
        assert_eq!(result.hits[1].library_index, 2);
        assert_eq!(state.total(), total_search_pairs(2, 3));
    }

    #[test]
    fn run_search_request_applies_taxonomic_reranking_before_final_top_n() {
        let request = crate::api::SearchRequest {
            query_source_label: "query.mgf".to_string(),
            query_mgf_text: Some(
                "BEGIN IONS\nNAME=q\nPEPMASS=100.0\n10 100\n20 80\n30 50\nEND IONS\n"
                    .to_string(),
            ),
            query_mgf_path: None,
            library_source_label: "library.mgf".to_string(),
            library_mgf_text: Some(
                concat!(
                    "BEGIN IONS\nNAME=withania\nPEPMASS=100.0\n",
                    "INCHIKEY=AAAAAAAAAAAAAA-111\n",
                    "10 100\n21 80\n35 30\nEND IONS\n",
                    "BEGIN IONS\nNAME=panax\nPEPMASS=100.0\n",
                    "INCHIKEY=BBBBBBBBBBBBBB-222\n",
                    "10 100\n20 80\n30 50\nEND IONS\n",
                )
                .to_string(),
            ),
            library_mgf_path: None,
            parse: crate::api::ParseConfig {
                identifier: "NAME".to_string(),
                query_identifier: None,
                library_identifier: None,
                min_peaks: 1,
                max_peaks: 1000,
                missing_precursor_mz: MissingPrecursorMz::Reject,
            },
            search: LibrarySearchParams {
                compute: base_compute_params(SimilarityMetric::LinearCosine),
                precursor_mz_tolerance: 0.1,
                min_matched_peaks: 1,
                min_similarity_threshold: 0.0,
                top_n: 1,
            },
            taxonomy: Some(crate::api::SearchTaxonomyRequest {
                query_text: "Withania somnifera".to_string(),
                lotus_source_label: "lotus.csv".to_string(),
                lotus_csv_text: Some(
                    concat!(
                        "structure_inchikey,organism_wikidata,organism_name,organism_taxonomy_01domain,organism_taxonomy_02kingdom,organism_taxonomy_03phylum,organism_taxonomy_04class,organism_taxonomy_05order,organism_taxonomy_06family,organism_taxonomy_07tribe,organism_taxonomy_08genus,organism_taxonomy_09species,organism_taxonomy_10varietas\n",
                        "\"AAAAAAAAAAAAAA-111\",http://www.wikidata.org/entity/Q1,\"Withania somnifera\",Eukaryota,Archaeplastida,Streptophyta,Magnoliopsida,Solanales,Solanaceae,NA,Withania,Withania somnifera,NA\n",
                        "\"BBBBBBBBBBBBBB-222\",http://www.wikidata.org/entity/Q2,\"Panax ginseng\",Eukaryota,Archaeplastida,Streptophyta,Magnoliopsida,Apiales,Araliaceae,NA,Panax,Panax ginseng,NA\n",
                    )
                    .to_string(),
                ),
                lotus_csv_path: None,
            }),
            query_key: Some(SearchQueryKey::FeatureId),
        };

        let artifact = run_search_request(request).expect("search request");

        assert!(artifact.result.taxonomic_reranking_applied);
        assert_eq!(
            artifact.result.taxonomic_query.as_deref(),
            Some("Withania somnifera")
        );
        assert_eq!(artifact.result.hits.len(), 1);
        assert_eq!(artifact.result.hits[0].library_index, 0);
        assert_eq!(artifact.result.hits[0].taxonomic_score, 9.0);
        assert_eq!(
            artifact.result.hits[0].matched_organism_name.as_deref(),
            Some("Withania somnifera")
        );
    }

    #[test]
    fn run_search_request_accepts_genus_level_taxonomic_queries() {
        let request = crate::api::SearchRequest {
            query_source_label: "query.mgf".to_string(),
            query_mgf_text: Some(
                "BEGIN IONS\nNAME=q\nPEPMASS=100.0\n10 100\n20 80\n30 50\nEND IONS\n"
                    .to_string(),
            ),
            query_mgf_path: None,
            library_source_label: "library.mgf".to_string(),
            library_mgf_text: Some(
                concat!(
                    "BEGIN IONS\nNAME=withania\nPEPMASS=100.0\n",
                    "INCHIKEY=AAAAAAAAAAAAAA-111\n",
                    "10 100\n21 80\n35 30\nEND IONS\n",
                    "BEGIN IONS\nNAME=panax\nPEPMASS=100.0\n",
                    "INCHIKEY=BBBBBBBBBBBBBB-222\n",
                    "10 100\n20 80\n30 50\nEND IONS\n",
                )
                .to_string(),
            ),
            library_mgf_path: None,
            parse: crate::api::ParseConfig {
                identifier: "NAME".to_string(),
                query_identifier: None,
                library_identifier: None,
                min_peaks: 1,
                max_peaks: 1000,
                missing_precursor_mz: MissingPrecursorMz::Reject,
            },
            search: LibrarySearchParams {
                compute: base_compute_params(SimilarityMetric::LinearCosine),
                precursor_mz_tolerance: 0.1,
                min_matched_peaks: 1,
                min_similarity_threshold: 0.0,
                top_n: 1,
            },
            taxonomy: Some(crate::api::SearchTaxonomyRequest {
                query_text: "Withania".to_string(),
                lotus_source_label: "lotus.csv".to_string(),
                lotus_csv_text: Some(
                    concat!(
                        "structure_inchikey,organism_wikidata,organism_name,organism_taxonomy_01domain,organism_taxonomy_02kingdom,organism_taxonomy_03phylum,organism_taxonomy_04class,organism_taxonomy_05order,organism_taxonomy_06family,organism_taxonomy_07tribe,organism_taxonomy_08genus,organism_taxonomy_09species,organism_taxonomy_10varietas\n",
                        "\"AAAAAAAAAAAAAA-111\",http://www.wikidata.org/entity/Q1,\"Withania somnifera\",Eukaryota,Archaeplastida,Streptophyta,Magnoliopsida,Solanales,Solanaceae,NA,Withania,Withania somnifera,NA\n",
                        "\"BBBBBBBBBBBBBB-222\",http://www.wikidata.org/entity/Q2,\"Panax ginseng\",Eukaryota,Archaeplastida,Streptophyta,Magnoliopsida,Apiales,Araliaceae,NA,Panax,Panax ginseng,NA\n",
                    )
                    .to_string(),
                ),
                lotus_csv_path: None,
            }),
            query_key: Some(SearchQueryKey::FeatureId),
        };

        let artifact = run_search_request(request).expect("search request");

        assert!(artifact.result.taxonomic_reranking_applied);
        assert_eq!(artifact.result.taxonomic_query.as_deref(), Some("Withania"));
        assert_eq!(artifact.result.hits.len(), 1);
        assert_eq!(artifact.result.hits[0].library_index, 0);
        assert_eq!(artifact.result.hits[0].rank, 1);
        assert_eq!(artifact.result.hits[0].rank_before_taxonomy, Some(2));
        assert_eq!(artifact.result.hits[0].taxonomic_score, 8.0);
        assert_eq!(
            artifact.result.hits[0].matched_shared_rank.as_deref(),
            Some("genus")
        );
    }

    #[test]
    fn search_candidate_sorting_uses_score_matches_then_library_index() {
        let result = SearchResult {
            hits: finalize_search_candidates(
                vec![
                    super::SearchCandidate {
                        query_index: 0,
                        library_index: 2,
                        rank_before_taxonomy: 3,
                        spectral_score: 0.8,
                        taxonomic_score: 0.0,
                        combined_score: 0.8,
                        matches: 4,
                        matched_organism_name: None,
                        matched_organism_wikidata: None,
                        matched_shared_rank: None,
                        matched_short_inchikey: None,
                    },
                    super::SearchCandidate {
                        query_index: 0,
                        library_index: 1,
                        rank_before_taxonomy: 2,
                        spectral_score: 0.8,
                        taxonomic_score: 0.0,
                        combined_score: 0.8,
                        matches: 5,
                        matched_organism_name: None,
                        matched_organism_wikidata: None,
                        matched_shared_rank: None,
                        matched_short_inchikey: None,
                    },
                    super::SearchCandidate {
                        query_index: 0,
                        library_index: 0,
                        rank_before_taxonomy: 1,
                        spectral_score: 0.8,
                        taxonomic_score: 0.0,
                        combined_score: 0.8,
                        matches: 5,
                        matched_organism_name: None,
                        matched_organism_wikidata: None,
                        matched_shared_rank: None,
                        matched_short_inchikey: None,
                    },
                ],
                3,
            ),
            query_count: 1,
            library_count: 3,
            metric: SimilarityMetric::LinearCosine,
        };

        assert_eq!(
            result
                .hits
                .iter()
                .map(|hit| hit.library_index)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn search_respects_precursor_mz_tolerance() {
        let queries = vec![spectrum(0, 100.0, &[(10.0, 1.0), (20.0, 0.8), (30.0, 0.5)])];
        let library = vec![
            spectrum(10, 100.03, &[(10.0, 1.0), (20.0, 0.8), (30.0, 0.5)]),
            spectrum(11, 100.20, &[(10.0, 1.0), (20.0, 0.8), (30.0, 0.5)]),
        ];
        let result = search_library(
            queries,
            library,
            LibrarySearchParams {
                compute: base_compute_params(SimilarityMetric::LinearCosine),
                precursor_mz_tolerance: 0.05,
                min_matched_peaks: 2,
                min_similarity_threshold: 0.1,
                top_n: 5,
            },
        )
        .expect("search should succeed");

        assert_eq!(result.hits.len(), 1);
        assert_eq!(result.hits[0].library_index, 0);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_search_matches_incremental_result_shape() {
        let queries = vec![spectrum(0, 100.0, &[(10.0, 1.0), (20.0, 0.8), (30.0, 0.5)])];
        let library = vec![
            spectrum(10, 101.0, &[(10.0, 1.0), (20.0, 0.8), (30.0, 0.5)]),
            spectrum(11, 102.0, &[(10.0, 1.0), (20.1, 0.7), (31.0, 0.4)]),
        ];
        let params = LibrarySearchParams {
            compute: base_compute_params(SimilarityMetric::LinearCosine),
            precursor_mz_tolerance: 5.0,
            min_matched_peaks: 2,
            min_similarity_threshold: 0.1,
            top_n: 2,
        };

        let handle = start_native_search(queries.clone(), library.clone(), params.clone());
        let native = loop {
            if let Some(message) = handle.try_recv() {
                match message {
                    super::SearchMessage::Finished(result) => break result,
                    super::SearchMessage::Cancelled => panic!("unexpected cancel"),
                    super::SearchMessage::Failed(err) => panic!("native search failed: {err}"),
                }
            }
            std::thread::yield_now();
        };

        let mut incremental =
            IncrementalSearchState::new(queries, library, params).expect("search state");
        let incremental = loop {
            match incremental.step(32).expect("incremental step") {
                IncrementalSearchStep::Progress => {}
                IncrementalSearchStep::Finished(result) => break result,
                IncrementalSearchStep::Cancelled => panic!("unexpected cancel"),
            }
        };

        assert_eq!(native, incremental);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn indexed_network_neighbors_match_scalar_linear_cosine() {
        let spectra = vec![
            spectrum(0, 100.0, &[(10.0, 1.0), (20.0, 0.8), (30.0, 0.5)]),
            spectrum(1, 100.0, &[(10.0, 1.0), (20.0, 0.8), (30.0, 0.5)]),
            spectrum(2, 100.0, &[(10.0, 1.0), (20.0, 0.5), (40.0, 0.2)]),
        ];
        let compute = base_compute_params(SimilarityMetric::LinearCosine);
        let scorer = MetricScorer::new(compute).expect("scalar scorer");
        let index = MetricIndex::new(compute, &spectra)
            .expect("linear cosine should support index")
            .expect("index build");
        let total = super::total_network_pairs(spectra.len());

        let scalar = super::score_network_neighbors_parallel(
            &spectra,
            &scorer,
            0.1,
            2,
            2,
            total,
            Arc::new(AtomicUsize::new(0)),
            &|_, _, _| {},
            &|| false,
        )
        .expect("scalar network scoring");
        let indexed = super::score_network_neighbors_indexed_parallel(
            &spectra,
            &index,
            0.1,
            2,
            2,
            total,
            Arc::new(AtomicUsize::new(0)),
            &|_, _, _| {},
            &|| false,
        )
        .expect("indexed network scoring");

        assert_selected_neighbors_eq(&indexed, &scalar);
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn assert_selected_neighbors_eq(
        left: &[Vec<crate::network::SelectedNeighbor>],
        right: &[Vec<crate::network::SelectedNeighbor>],
    ) {
        assert_eq!(left.len(), right.len());
        for (left_neighbors, right_neighbors) in left.iter().zip(right) {
            assert_eq!(left_neighbors.len(), right_neighbors.len());
            for (left, right) in left_neighbors.iter().zip(right_neighbors) {
                assert_eq!(left.neighbor, right.neighbor);
                assert_eq!(left.matches, right.matches);
                assert!((left.score - right.score).abs() < 1e-10);
            }
        }
    }
}
