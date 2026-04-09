//! Similarity metric configuration, scorer construction, and metric-specific preprocessing.

use std::sync::Arc;

use mass_spectrometry::prelude::{
    GenericSpectrum, HungarianCosine, LinearCosine, LinearEntropy, ModifiedHungarianCosine,
    ModifiedLinearCosine, ModifiedLinearEntropy, MsEntropyCleanSpectrum, ScalarSimilarity,
    SiriusMergeClosePeaks, SpectralProcessor, SpectrumAlloc, SpectrumMut,
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
                "Linear-time cosine; requires well-separated peaks and returns an error otherwise."
            }
            Self::ModifiedHungarianCosine => {
                "Precursor-shift-aware cosine using Hungarian assignment for analog-style matching."
            }
            Self::ModifiedLinearCosine => {
                "Linear-time precursor-shift-aware cosine; requires well-separated peaks."
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

impl MetricScorer {
    /// Builds a configured scorer for the requested metric and weighting parameters.
    pub fn new(params: ComputeParams) -> Result<Self, String> {
        let inner = match params.metric {
            SimilarityMetric::HungarianCosine => {
                HungarianCosine::new(
                    params.mz_power,
                    params.intensity_power,
                    params.fragment_mz_tolerance,
                )
                .map(MetricScorerInner::HungarianCosine)
            }
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
        result.map_err(|err| {
            format!(
                "{} failed for pair ({left_idx},{right_idx}): {err:?}",
                self.metric.label()
            )
        })
    }
}

/// Applies any metric-specific preprocessing needed before scoring starts.
pub fn preprocess_spectra_for_metric<T>(
    spectra: Vec<SpectrumRecord<T>>,
    params: ComputeParams,
) -> Result<Vec<SpectrumRecord<T>>, String> {
    let spectra = apply_top_n_peak_filter(spectra, params.top_n_peaks)?;

    if !params.metric.needs_linear_entropy_preprocessing() {
        return Ok(spectra);
    }

    let cleaner = MsEntropyCleanSpectrum::builder()
        .build()
        .map_err(|err| format!("failed to configure ms_entropy cleaner: {err:?}"))?;
    let merger = SiriusMergeClosePeaks::new(params.fragment_mz_tolerance)
        .map_err(|err| format!("failed to configure close-peak merger: {err:?}"))?;

    Ok(spectra
        .into_iter()
        .map(|mut record| {
            let cleaned = cleaner.process(record.spectrum.as_ref());
            let merged = merger.process(&cleaned);
            record.spectrum = Arc::new(merged);
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

    use mass_spectrometry::prelude::{GenericSpectrum, SpectrumAlloc, SpectrumMut};

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
    fn linear_cosine_surfaces_invalid_peak_spacing() {
        let left = record(0, &[(10.0, 1.0), (10.15, 0.5)]);
        let right = record(1, &[(10.0, 1.0), (10.15, 0.5)]);
        let scorer = MetricScorer::new(ComputeParams {
            metric: SimilarityMetric::LinearCosine,
            fragment_mz_tolerance: 0.1,
            mz_power: 0.0,
            intensity_power: 1.0,
            top_n_peaks: None,
        })
        .expect("scorer");

        let err = scorer
            .similarity(left.spectrum.as_ref(), right.spectrum.as_ref(), 0, 1)
            .expect_err("invalid spacing should error");
        assert!(err.contains("LinearCosine failed"));
    }

    #[test]
    fn modified_linear_cosine_surfaces_invalid_peak_spacing() {
        let left = record(0, &[(10.0, 1.0), (10.15, 0.5)]);
        let right = record(1, &[(10.0, 1.0), (10.15, 0.5)]);
        let scorer = MetricScorer::new(ComputeParams {
            metric: SimilarityMetric::ModifiedLinearCosine,
            fragment_mz_tolerance: 0.1,
            mz_power: 0.0,
            intensity_power: 1.0,
            top_n_peaks: None,
        })
        .expect("scorer");

        let err = scorer
            .similarity(left.spectrum.as_ref(), right.spectrum.as_ref(), 0, 1)
            .expect_err("invalid spacing should error");
        assert!(err.contains("ModifiedLinearCosine failed"));
    }
}
