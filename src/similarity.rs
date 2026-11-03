//! Similarity metric configuration, scorer construction, and metric-specific preprocessing.

use std::sync::Arc;

use mass_spectrometry::prelude::{
    FlashCosineIndex, FlashEntropyIndex, FlashSearchResult, GenericSpectrum, HungarianCosine,
    LinearCosine, LinearEntropy, ModifiedHungarianCosine, ModifiedLinearCosine,
    ModifiedLinearEntropy, MsEntropyCleanSpectrum, PepmassFilter, ScalarSimilarity, SearchState,
    SimilarityComputationError, SiriusMergeClosePeaks, SpectraIndexBuilder, SpectralProcessor,
    SpectrumAlloc, SpectrumMut,
};
use serde::{Deserialize, Serialize};

use crate::model::SpectrumRecord;

/// Supported spectral-similarity metrics exposed by the matcher.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SimilarityMetric {
    #[default]
    HungarianCosine,
    LinearCosine,
    ModifiedHungarianCosine,
    ModifiedLinearCosine,
    LinearEntropyWeighted,
    LinearEntropyUnweighted,
    ModifiedLinearEntropyWeighted,
    ModifiedLinearEntropyUnweighted,
}

impl SimilarityMetric {
    /// All supported metric values in CLI/API display order.
    pub const ALL: [Self; 8] = [
        Self::HungarianCosine,
        Self::LinearCosine,
        Self::ModifiedHungarianCosine,
        Self::ModifiedLinearCosine,
        Self::LinearEntropyWeighted,
        Self::LinearEntropyUnweighted,
        Self::ModifiedLinearEntropyWeighted,
        Self::ModifiedLinearEntropyUnweighted,
    ];

    /// Stable string label used in configs, JSON artifacts, and CLI output.
    pub fn label(self) -> &'static str {
        match self {
            Self::HungarianCosine => "HungarianCosine",
            Self::LinearCosine => "LinearCosine",
            Self::ModifiedHungarianCosine => "ModifiedHungarianCosine",
            Self::ModifiedLinearCosine => "ModifiedLinearCosine",
            Self::LinearEntropyWeighted => "LinearEntropyWeighted",
            Self::LinearEntropyUnweighted => "LinearEntropyUnweighted",
            Self::ModifiedLinearEntropyWeighted => "ModifiedLinearEntropyWeighted",
            Self::ModifiedLinearEntropyUnweighted => "ModifiedLinearEntropyUnweighted",
        }
    }

    /// Short user-facing description of the metric.
    pub fn description(self) -> &'static str {
        match self {
            Self::HungarianCosine => {
                "Exact cosine matching with Hungarian assignment; slower, more exhaustive."
            }
            Self::LinearCosine => {
                "Linear-time cosine after close-peak merging; compatible with Flash indexing."
            }
            Self::ModifiedHungarianCosine => {
                "Precursor-shift-aware cosine using Hungarian assignment for analog-style matching."
            }
            Self::ModifiedLinearCosine => {
                "Linear-time precursor-shift-aware cosine after close-peak merging; compatible with Flash indexing."
            }
            Self::LinearEntropyWeighted => {
                "Spectral entropy similarity with intensity weighting after entropy preprocessing."
            }
            Self::LinearEntropyUnweighted => {
                "Spectral entropy similarity without intensity weighting after entropy preprocessing."
            }
            Self::ModifiedLinearEntropyWeighted => {
                "Precursor-shift-aware spectral entropy similarity with intensity weighting."
            }
            Self::ModifiedLinearEntropyUnweighted => {
                "Precursor-shift-aware spectral entropy similarity without intensity weighting."
            }
        }
    }

    /// Returns whether the metric requires entropy-specific spectrum cleanup before scoring.
    fn needs_linear_entropy_preprocessing(self) -> bool {
        matches!(
            self,
            Self::LinearEntropyWeighted
                | Self::LinearEntropyUnweighted
                | Self::ModifiedLinearEntropyWeighted
                | Self::ModifiedLinearEntropyUnweighted
        )
    }

    /// Returns whether the metric needs close-peak merging for linear matching/indexing.
    fn needs_close_peak_merging(self) -> bool {
        matches!(
            self,
            Self::LinearCosine
                | Self::ModifiedLinearCosine
                | Self::LinearEntropyWeighted
                | Self::LinearEntropyUnweighted
                | Self::ModifiedLinearEntropyWeighted
                | Self::ModifiedLinearEntropyUnweighted
        )
    }
}

/// Parameters controlling fragment-level similarity computation.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct ComputeParams {
    /// Similarity metric used for scoring.
    pub metric: SimilarityMetric,
    /// Fragment m/z matching tolerance in Dalton.
    pub fragment_mz_tolerance: f64,
    /// Exponent applied to m/z while weighting peaks.
    pub mz_power: f64,
    /// Exponent applied to intensity while weighting peaks.
    pub intensity_power: f64,
    /// Optional cap on the number of peaks retained for similarity computation only.
    #[serde(default)]
    pub top_n_peaks: Option<usize>,
}

enum MetricScorerInner {
    HungarianCosine(HungarianCosine),
    LinearCosine(LinearCosine),
    ModifiedHungarianCosine(ModifiedHungarianCosine),
    ModifiedLinearCosine(ModifiedLinearCosine),
    LinearEntropy(LinearEntropy),
    ModifiedLinearEntropy(ModifiedLinearEntropy),
}

/// Concrete scorer wrapper that hides backend-specific matcher types.
pub struct MetricScorer {
    metric: SimilarityMetric,
    inner: MetricScorerInner,
}

enum MetricIndexInner {
    Cosine(FlashCosineIndex<f64>),
    ModifiedCosine(FlashCosineIndex<f64>),
    Entropy(FlashEntropyIndex<f64>),
    ModifiedEntropy(FlashEntropyIndex<f64>),
}

pub enum MetricIndexSearchState {
    Cosine(SearchState),
    Entropy(SearchState),
}

/// Similarity result returned by an indexed library-scale search.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IndexedSimilarity {
    /// Zero-based index of the matched spectrum in the indexed library.
    pub library_index: usize,
    /// Similarity score in the same range as scalar metric scoring.
    pub score: f64,
    /// Number of matched fragment peaks.
    pub matches: usize,
}

/// Library-scale inverted index for metrics supported by mass-spectrometry-traits.
pub struct MetricIndex {
    metric: SimilarityMetric,
    inner: MetricIndexInner,
}

impl MetricIndex {
    /// Builds an index for metrics with a compatible flash backend.
    pub fn new(params: ComputeParams, library: &[SpectrumRecord]) -> Option<Result<Self, String>> {
        Self::build_with_filter(params, library, PepmassFilter::disabled())
    }

    /// Builds a search index that prunes incompatible precursors before fragment scoring.
    pub(crate) fn new_for_search(
        params: ComputeParams,
        library: &[SpectrumRecord],
        precursor_mz_tolerance: f64,
    ) -> Option<Result<Self, String>> {
        let filter = PepmassFilter::within_tolerance(precursor_mz_tolerance).unwrap_or_default();
        Self::build_with_filter(params, library, filter)
    }

    fn build_with_filter(
        params: ComputeParams,
        library: &[SpectrumRecord],
        filter: PepmassFilter,
    ) -> Option<Result<Self, String>> {
        if matches!(
            params.metric,
            SimilarityMetric::HungarianCosine | SimilarityMetric::ModifiedHungarianCosine
        ) {
            return None;
        }
        let spectra = library
            .iter()
            .map(|record| record.spectrum.as_ref().clone())
            .collect::<Vec<_>>();
        let build = match params.metric {
            SimilarityMetric::LinearCosine => FlashCosineIndex::<f64>::builder()
                .mz_power(params.mz_power)
                .intensity_power(params.intensity_power)
                .mz_tolerance(params.fragment_mz_tolerance)
                .pepmass_filter(filter)
                .build(&spectra)
                .map(MetricIndexInner::Cosine)
                .map_err(|err| format!("failed to build LinearCosine flash index: {err:?}")),
            SimilarityMetric::ModifiedLinearCosine => FlashCosineIndex::<f64>::builder()
                .mz_power(params.mz_power)
                .intensity_power(params.intensity_power)
                .mz_tolerance(params.fragment_mz_tolerance)
                .pepmass_filter(filter)
                .build(&spectra)
                .map(MetricIndexInner::ModifiedCosine)
                .map_err(|err| {
                    format!("failed to build ModifiedLinearCosine flash index: {err:?}")
                }),
            SimilarityMetric::LinearEntropyWeighted => FlashEntropyIndex::<f64>::builder()
                .mz_power(params.mz_power)
                .intensity_power(params.intensity_power)
                .mz_tolerance(params.fragment_mz_tolerance)
                .weighted(true)
                .pepmass_filter(filter)
                .build(&spectra)
                .map(MetricIndexInner::Entropy)
                .map_err(|err| {
                    format!("failed to build LinearEntropyWeighted flash index: {err:?}")
                }),
            SimilarityMetric::LinearEntropyUnweighted => FlashEntropyIndex::<f64>::builder()
                .mz_power(params.mz_power)
                .intensity_power(params.intensity_power)
                .mz_tolerance(params.fragment_mz_tolerance)
                .weighted(false)
                .pepmass_filter(filter)
                .build(&spectra)
                .map(MetricIndexInner::Entropy)
                .map_err(|err| {
                    format!("failed to build LinearEntropyUnweighted flash index: {err:?}")
                }),
            SimilarityMetric::ModifiedLinearEntropyWeighted => FlashEntropyIndex::<f64>::builder()
                .mz_power(params.mz_power)
                .intensity_power(params.intensity_power)
                .mz_tolerance(params.fragment_mz_tolerance)
                .weighted(true)
                .pepmass_filter(filter)
                .build(&spectra)
                .map(MetricIndexInner::ModifiedEntropy)
                .map_err(|err| {
                    format!("failed to build ModifiedLinearEntropyWeighted flash index: {err:?}")
                }),
            SimilarityMetric::ModifiedLinearEntropyUnweighted => {
                FlashEntropyIndex::<f64>::builder()
                    .mz_power(params.mz_power)
                    .intensity_power(params.intensity_power)
                    .mz_tolerance(params.fragment_mz_tolerance)
                    .weighted(false)
                    .pepmass_filter(filter)
                    .build(&spectra)
                    .map(MetricIndexInner::ModifiedEntropy)
                    .map_err(|err| {
                        format!(
                            "failed to build ModifiedLinearEntropyUnweighted flash index: {err:?}"
                        )
                    })
            }
            SimilarityMetric::HungarianCosine | SimilarityMetric::ModifiedHungarianCosine => {
                unreachable!("Hungarian metrics have no Flash index")
            }
        };

        Some(build.map(|inner| Self {
            metric: params.metric,
            inner,
        }))
    }

    /// Creates reusable per-query scratch storage for this index.
    pub fn new_search_state(&self) -> MetricIndexSearchState {
        match &self.inner {
            MetricIndexInner::Cosine(index) | MetricIndexInner::ModifiedCosine(index) => {
                MetricIndexSearchState::Cosine(index.new_search_state())
            }
            MetricIndexInner::Entropy(index) | MetricIndexInner::ModifiedEntropy(index) => {
                MetricIndexSearchState::Entropy(index.new_search_state())
            }
        }
    }

    /// Searches the indexed library for spectra that share at least one peak.
    pub fn search_with_state(
        &self,
        query: &SpectrumRecord,
        state: &mut MetricIndexSearchState,
        query_idx: usize,
    ) -> Result<Vec<IndexedSimilarity>, String> {
        let result = match (&self.inner, state) {
            (MetricIndexInner::Cosine(index), MetricIndexSearchState::Cosine(state)) => index
                .search_with_state(query.spectrum.as_ref(), state)
                .map_err(|err| self.format_index_error(err, query_idx))?,
            (MetricIndexInner::ModifiedCosine(index), MetricIndexSearchState::Cosine(state)) => {
                index
                    .search_modified_with_state(query.spectrum.as_ref(), state)
                    .map_err(|err| self.format_index_error(err, query_idx))?
            }
            (MetricIndexInner::Entropy(index), MetricIndexSearchState::Entropy(state)) => index
                .search_with_state(query.spectrum.as_ref(), state)
                .map_err(|err| self.format_index_error(err, query_idx))?,
            (MetricIndexInner::ModifiedEntropy(index), MetricIndexSearchState::Entropy(state)) => {
                index
                    .search_modified_with_state(query.spectrum.as_ref(), state)
                    .map_err(|err| self.format_index_error(err, query_idx))?
            }
            _ => return Err("flash index search state does not match metric".to_string()),
        };

        Ok(result.into_iter().map(indexed_similarity).collect())
    }

    fn format_index_error(&self, err: SimilarityComputationError, query_idx: usize) -> String {
        format!(
            "{} flash index search failed for query {query_idx}: {err:?}",
            self.metric.label()
        )
    }
}

fn indexed_similarity(result: FlashSearchResult) -> IndexedSimilarity {
    IndexedSimilarity {
        library_index: result.spectrum_id as usize,
        score: result.score,
        matches: result.n_matches,
    }
}

impl MetricScorer {
    /// Builds a configured scorer for the requested metric and weighting parameters.
    pub fn new(params: ComputeParams) -> Result<Self, String> {
        let inner = match params.metric {
            SimilarityMetric::HungarianCosine => HungarianCosine::new(
                params.mz_power,
                params.intensity_power,
                params.fragment_mz_tolerance,
            )
            .map(MetricScorerInner::HungarianCosine),
            SimilarityMetric::LinearCosine => LinearCosine::new(
                params.mz_power,
                params.intensity_power,
                params.fragment_mz_tolerance,
            )
            .map(MetricScorerInner::LinearCosine),
            SimilarityMetric::ModifiedHungarianCosine => ModifiedHungarianCosine::new(
                params.mz_power,
                params.intensity_power,
                params.fragment_mz_tolerance,
            )
            .map(MetricScorerInner::ModifiedHungarianCosine),
            SimilarityMetric::ModifiedLinearCosine => ModifiedLinearCosine::new(
                params.mz_power,
                params.intensity_power,
                params.fragment_mz_tolerance,
            )
            .map(MetricScorerInner::ModifiedLinearCosine),
            SimilarityMetric::LinearEntropyWeighted => LinearEntropy::new(
                params.mz_power,
                params.intensity_power,
                params.fragment_mz_tolerance,
                true,
            )
            .map(MetricScorerInner::LinearEntropy),
            SimilarityMetric::LinearEntropyUnweighted => LinearEntropy::new(
                params.mz_power,
                params.intensity_power,
                params.fragment_mz_tolerance,
                false,
            )
            .map(MetricScorerInner::LinearEntropy),
            SimilarityMetric::ModifiedLinearEntropyWeighted => ModifiedLinearEntropy::new(
                params.mz_power,
                params.intensity_power,
                params.fragment_mz_tolerance,
                true,
            )
            .map(MetricScorerInner::ModifiedLinearEntropy),
            SimilarityMetric::ModifiedLinearEntropyUnweighted => ModifiedLinearEntropy::new(
                params.mz_power,
                params.intensity_power,
                params.fragment_mz_tolerance,
                false,
            )
            .map(MetricScorerInner::ModifiedLinearEntropy),
        }
        .map_err(|err| format!("failed to configure {}: {err:?}", params.metric.label()))?;

        Ok(Self {
            metric: params.metric,
            inner,
        })
    }

    /// Computes a similarity score and matched-peak count for a spectrum pair.
    pub fn similarity(
        &self,
        left: &GenericSpectrum,
        right: &GenericSpectrum,
        left_idx: usize,
        right_idx: usize,
    ) -> Result<(f64, usize), String> {
        let result = match &self.inner {
            MetricScorerInner::HungarianCosine(sim) => sim.similarity(left, right),
            MetricScorerInner::LinearCosine(sim) => sim.similarity(left, right),
            MetricScorerInner::ModifiedHungarianCosine(sim) => sim.similarity(left, right),
            MetricScorerInner::ModifiedLinearCosine(sim) => sim.similarity(left, right),
            MetricScorerInner::LinearEntropy(sim) => sim.similarity(left, right),
            MetricScorerInner::ModifiedLinearEntropy(sim) => sim.similarity(left, right),
        };
        result.map_err(|err| self.format_similarity_error(err, left_idx, right_idx))
    }

    /// Formats backend similarity failures into a user-facing message.
    fn format_similarity_error(
        &self,
        err: SimilarityComputationError,
        left_idx: usize,
        right_idx: usize,
    ) -> String {
        match (self.metric, err) {
            (
                SimilarityMetric::LinearCosine | SimilarityMetric::ModifiedLinearCosine,
                SimilarityComputationError::InvalidPeakSpacing(side),
            ) => format!(
                "{} failed for pair ({left_idx},{right_idx}): InvalidPeakSpacing(\"{side}\"). \
This metric requires well-separated peaks (> 2 * fragment_mz_tolerance). \
Use HungarianCosine or ModifiedHungarianCosine for general-purpose scoring.",
                self.metric.label()
            ),
            (_, err) => format!(
                "{} failed for pair ({left_idx},{right_idx}): {err:?}",
                self.metric.label()
            ),
        }
    }
}

/// Applies any metric-specific preprocessing needed before scoring starts.
pub fn preprocess_spectra_for_metric<T>(
    spectra: Vec<SpectrumRecord<T>>,
    params: ComputeParams,
) -> Result<Vec<SpectrumRecord<T>>, String> {
    let mut spectra = apply_top_n_peak_filter(spectra, params.top_n_peaks)?;

    if params.metric.needs_linear_entropy_preprocessing() {
        let cleaner = MsEntropyCleanSpectrum::builder()
            .build()
            .map_err(|err| format!("failed to configure ms_entropy cleaner: {err:?}"))?;

        spectra = spectra
            .into_iter()
            .map(|mut record| {
                record.spectrum = Arc::new(cleaner.process(record.spectrum.as_ref()));
                record
            })
            .collect();
    }

    if !params.metric.needs_close_peak_merging() {
        return Ok(spectra);
    }

    let merger = SiriusMergeClosePeaks::new(params.fragment_mz_tolerance)
        .map_err(|err| format!("failed to configure close-peak merger: {err:?}"))?;

    Ok(spectra
        .into_iter()
        .map(|mut record| {
            record.spectrum = Arc::new(merger.process(record.spectrum.as_ref()));
            record
        })
        .collect())
}

/// Applies an optional top-N-by-intensity filter to spectra used for scoring.
fn apply_top_n_peak_filter<T>(
    spectra: Vec<SpectrumRecord<T>>,
    top_n_peaks: Option<usize>,
) -> Result<Vec<SpectrumRecord<T>>, String> {
    let Some(limit) = top_n_peaks.filter(|limit| *limit > 0) else {
        return Ok(spectra);
    };

    spectra
        .into_iter()
        .map(|mut record| {
            if record.peaks.len() <= limit {
                return Ok(record);
            }

            let mut selected = record.peaks.as_ref().clone();
            selected.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.total_cmp(&b.0)));
            selected.truncate(limit);
            selected.sort_by(|a, b| a.0.total_cmp(&b.0));

            let mut spectrum =
                GenericSpectrum::with_capacity(record.meta.precursor_mz, selected.len()).map_err(
                    |err| {
                        format!(
                            "failed to allocate top-{limit} filtered spectrum for node {}: {err:?}",
                            record.meta.id
                        )
                    },
                )?;
            for (mz, intensity) in &selected {
                spectrum.add_peak(*mz, *intensity).map_err(|err| {
                    format!(
                        "failed to add top-{limit} filtered peak for node {}: {err:?}",
                        record.meta.id
                    )
                })?;
            }
            record.spectrum = Arc::new(spectrum);
            Ok(record)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use mass_spectrometry::prelude::{GenericSpectrum, Spectrum, SpectrumAlloc, SpectrumMut};

    use super::{ComputeParams, MetricScorer, SimilarityMetric, preprocess_spectra_for_metric};
    use crate::model::{SpectrumMetadata, SpectrumRecord};

    fn record(id: usize, peaks: &[(f64, f64)]) -> SpectrumRecord {
        let mut sorted_peaks = peaks.to_vec();
        sorted_peaks.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut spectrum =
            GenericSpectrum::with_capacity(100.0, sorted_peaks.len()).expect("alloc test spectrum");
        for (mz, intensity) in &sorted_peaks {
            spectrum.add_peak(*mz, *intensity).expect("add test peak");
        }
        SpectrumRecord {
            meta: SpectrumMetadata {
                id,
                spectrum_id: format!("feature_{id}"),
                label: format!("s{id}"),
                raw_name: format!("raw{id}"),
                feature_id: None,
                scans: None,
                filename: None,
                source_scan_usi: None,
                featurelist_feature_id: None,
                headers: BTreeMap::new(),
                precursor_mz: 100.0,
                num_peaks: peaks.len(),
            },
            peaks: Arc::new(peaks.to_vec()),
            spectrum: Arc::new(spectrum),
            payload: (),
        }
    }

    #[test]
    fn top_n_peak_filter_keeps_most_intense_peaks_for_similarity_only() {
        let spectra = vec![record(
            0,
            &[(50.0, 0.4), (10.0, 1.0), (30.0, 0.8), (20.0, 0.2)],
        )];
        let processed = preprocess_spectra_for_metric(
            spectra,
            ComputeParams {
                metric: SimilarityMetric::LinearCosine,
                fragment_mz_tolerance: 0.2,
                mz_power: 0.0,
                intensity_power: 1.0,
                top_n_peaks: Some(2),
            },
        )
        .expect("preprocess");
        let reference = record(1, &[(10.0, 1.0), (30.0, 0.8)]);
        let scorer = MetricScorer::new(ComputeParams {
            metric: SimilarityMetric::LinearCosine,
            fragment_mz_tolerance: 0.2,
            mz_power: 0.0,
            intensity_power: 1.0,
            top_n_peaks: None,
        })
        .expect("scorer");
        let (score, matches) = scorer
            .similarity(
                processed[0].spectrum.as_ref(),
                reference.spectrum.as_ref(),
                0,
                1,
            )
            .expect("similarity");

        assert_eq!(processed[0].peaks.as_ref().len(), 4);
        assert_eq!(matches, 2);
        assert!((score - 1.0).abs() < 1e-9);
    }

    #[test]
    fn linear_cosine_preprocessing_merges_close_peaks() {
        let params = ComputeParams {
            metric: SimilarityMetric::LinearCosine,
            fragment_mz_tolerance: 0.1,
            mz_power: 0.0,
            intensity_power: 1.0,
            top_n_peaks: None,
        };
        let processed = preprocess_spectra_for_metric(
            vec![
                record(0, &[(10.0, 1.0), (10.15, 0.5)]),
                record(1, &[(10.0, 1.0), (10.15, 0.5)]),
            ],
            params,
        )
        .expect("preprocess");
        let scorer = MetricScorer::new(params).expect("scorer");
        let (score, matches) = scorer
            .similarity(
                processed[0].spectrum.as_ref(),
                processed[1].spectrum.as_ref(),
                0,
                1,
            )
            .expect("merged spectra should score");

        assert_eq!(processed[0].peaks.as_ref().len(), 2);
        assert_eq!(processed[0].spectrum.as_ref().len(), 1);
        assert_eq!(matches, 1);
        assert!((score - 1.0).abs() < 1e-9);
    }

    #[test]
    fn modified_linear_cosine_preprocessing_merges_close_peaks() {
        let params = ComputeParams {
            metric: SimilarityMetric::ModifiedLinearCosine,
            fragment_mz_tolerance: 0.1,
            mz_power: 0.0,
            intensity_power: 1.0,
            top_n_peaks: None,
        };
        let processed = preprocess_spectra_for_metric(
            vec![
                record(0, &[(10.0, 1.0), (10.15, 0.5)]),
                record(1, &[(10.0, 1.0), (10.15, 0.5)]),
            ],
            params,
        )
        .expect("preprocess");
        let scorer = MetricScorer::new(params).expect("scorer");
        let (score, matches) = scorer
            .similarity(
                processed[0].spectrum.as_ref(),
                processed[1].spectrum.as_ref(),
                0,
                1,
            )
            .expect("merged spectra should score");

        assert_eq!(processed[0].peaks.as_ref().len(), 2);
        assert_eq!(processed[0].spectrum.as_ref().len(), 1);
        assert_eq!(matches, 1);
        assert!((score - 1.0).abs() < 1e-9);
    }
}
