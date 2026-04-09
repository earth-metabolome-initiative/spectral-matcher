//! Shared in-memory data structures used across parsing, scoring, export, and serving.

use std::collections::BTreeMap;
use std::sync::Arc;

use mass_spectrometry::prelude::GenericSpectrum;
use serde::{Deserialize, Serialize};

use crate::similarity::SimilarityMetric;

/// Aggregate counters describing what happened while parsing an MGF source.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseStats {
    /// Number of `BEGIN IONS` blocks encountered.
    pub ions_blocks: usize,
    /// Number of spectra accepted after parsing and filtering.
    pub accepted: usize,
    /// Spectra dropped because no preferred name could be resolved.
    pub dropped_missing_name: usize,
    /// Spectra dropped because no precursor m/z was available.
    pub dropped_missing_precursor_mz: usize,
    /// Spectra dropped for having too few retained peaks.
    pub dropped_too_few_peaks: usize,
    /// Spectra dropped for having too many retained peaks.
    pub dropped_too_many_peaks: usize,
    /// Peaks discarded because they had non-positive intensity.
    pub dropped_nonpositive_intensity_peaks: usize,
    /// Spectra dropped because duplicate m/z values remained after parsing.
    pub dropped_duplicate_mz: usize,
}

/// Exported metadata describing a parsed spectrum.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpectrumMetadata {
    /// Zero-based internal spectrum identifier.
    pub id: usize,
    /// Sanitized label used for display and graph export.
    pub label: String,
    /// Original preferred name selected during parsing.
    pub raw_name: String,
    /// Optional `FEATURE_ID` header.
    pub feature_id: Option<String>,
    /// Optional `SCANS` header.
    pub scans: Option<String>,
    /// Optional originating filename.
    pub filename: Option<String>,
    /// Optional source scan USI.
    pub source_scan_usi: Option<String>,
    /// Optional `FEATURELIST_FEATURE_ID` header.
    pub featurelist_feature_id: Option<String>,
    /// Additional retained source headers.
    pub headers: BTreeMap<String, String>,
    /// Parsed precursor m/z value.
    pub precursor_mz: f64,
    /// Number of retained peaks.
    pub num_peaks: usize,
}

/// In-memory spectrum record pairing metadata, peaks, and an allocated spectrum object.
#[derive(Clone)]
pub struct SpectrumRecord<T = ()> {
    /// Exportable spectrum metadata.
    pub meta: SpectrumMetadata,
    /// Original peak list as parsed from source.
    pub peaks: Arc<Vec<(f64, f64)>>,
    /// Spectrum object used by the scoring backends.
    pub spectrum: Arc<GenericSpectrum>,
    /// Workflow-specific payload carried alongside the spectrum.
    pub payload: T,
}

/// Collection of spectra loaded from a single source plus parse statistics.
#[derive(Clone)]
pub struct SpectrumCollection<T = ()> {
    /// Human-readable source label.
    pub source_label: String,
    /// Parsed and accepted spectra.
    pub spectra: Vec<SpectrumRecord<T>>,
    /// Aggregate parsing statistics.
    pub stats: ParseStats,
}

/// Convenience alias for a loaded spectrum collection without extra payloads.
pub type LoadedSpectra = SpectrumCollection<()>;

/// Ranked hit connecting a query spectrum to a library or neighbor spectrum.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CandidateHit<H = ()> {
    /// Zero-based query spectrum index.
    pub query_index: usize,
    /// Zero-based matched spectrum index.
    pub library_index: usize,
    /// One-based rank within the query result set.
    pub rank: usize,
    /// Spectral similarity score for the hit.
    pub spectral_score: f64,
    /// Number of matched fragment peaks.
    pub matches: usize,
    /// Optional workflow-specific payload.
    pub payload: H,
}

/// Common interface for hit-like search results.
pub trait HitLike {
    /// Additional payload carried by the hit.
    type Payload;

    /// Returns the zero-based query index.
    fn query_index(&self) -> usize;
    /// Returns the zero-based matched spectrum index.
    fn library_index(&self) -> usize;
    /// Returns the one-based rank within the query result set.
    fn rank(&self) -> usize;
    /// Returns the spectral score.
    fn spectral_score(&self) -> f64;
    /// Returns the fragment-match count.
    fn matches(&self) -> usize;
    /// Returns the attached payload.
    fn payload(&self) -> &Self::Payload;
}

impl<H> HitLike for CandidateHit<H> {
    type Payload = H;

    fn query_index(&self) -> usize {
        self.query_index
    }

    fn library_index(&self) -> usize {
        self.library_index
    }

    fn rank(&self) -> usize {
        self.rank
    }

    fn spectral_score(&self) -> f64 {
        self.spectral_score
    }

    fn matches(&self) -> usize {
        self.matches
    }

    fn payload(&self) -> &Self::Payload {
        &self.payload
    }
}

/// Minimal search result container returned by direct search functions.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchResult<H = ()> {
    /// Ranked hits across all query spectra.
    pub hits: Vec<CandidateHit<H>>,
    /// Number of query spectra considered.
    pub query_count: usize,
    /// Number of library spectra considered.
    pub library_count: usize,
    /// Metric used to compute spectral scores.
    pub metric: SimilarityMetric,
}
