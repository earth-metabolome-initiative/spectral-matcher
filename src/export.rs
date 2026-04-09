//! TSV and JSON export helpers for search and consensus results and browser downloads.

use std::collections::BTreeSet;

use serde::Serialize;

use crate::api::{ConsensusArtifactResult, ConsensusClass, SearchArtifactResult};
use crate::model::{SpectrumMetadata, SpectrumRecord};

/// Query identifier column used in TSV/JSON exports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
pub enum SearchQueryKey {
    FeatureId,
    FeaturelistFeatureId,
    Scans,
    RawName,
    Label,
    NodeId,
}

impl SearchQueryKey {
    /// All supported query identifier modes in display order.
    pub const ALL: [Self; 6] = [
        Self::FeatureId,
        Self::FeaturelistFeatureId,
        Self::Scans,
        Self::RawName,
        Self::Label,
        Self::NodeId,
    ];

    /// Stable export label used in TSV/JSON payloads.
    pub fn label(self) -> &'static str {
        match self {
            Self::FeatureId => "FEATURE_ID",
            Self::FeaturelistFeatureId => "FEATURELIST_FEATURE_ID",
            Self::Scans => "SCANS",
            Self::RawName => "raw_name",
            Self::Label => "label",
            Self::NodeId => "node_id",
        }
    }

    /// Returns the identifier value for the selected query-key mode.
    pub fn value_for<T>(self, record: &SpectrumRecord<T>) -> String {
        match self {
            Self::FeatureId => record.meta.feature_id.clone().unwrap_or_default(),
            Self::FeaturelistFeatureId => record
                .meta
                .featurelist_feature_id
                .clone()
                .unwrap_or_default(),
            Self::Scans => record.meta.scans.clone().unwrap_or_default(),
            Self::RawName => record.meta.raw_name.clone(),
            Self::Label => record.meta.label.clone(),
            Self::NodeId => record.meta.id.to_string(),
        }
    }

    /// Returns the identifier value for the selected query-key mode using exported metadata.
    pub fn value_for_meta(self, meta: &SpectrumMetadata) -> String {
        match self {
            Self::FeatureId => meta.feature_id.clone().unwrap_or_default(),
            Self::FeaturelistFeatureId => meta.featurelist_feature_id.clone().unwrap_or_default(),
            Self::Scans => meta.scans.clone().unwrap_or_default(),
            Self::RawName => meta.raw_name.clone(),
            Self::Label => meta.label.clone(),
            Self::NodeId => meta.id.to_string(),
        }
    }
}

/// Renders a search artifact as a TSV table suitable for spreadsheet-style workflows.
pub fn export_search_tsv<TQ, TL>(
    result: &SearchArtifactResult,
    queries: &[SpectrumRecord<TQ>],
    library: &[SpectrumRecord<TL>],
    query_key: SearchQueryKey,
) -> String {
    let include_taxonomy = result.taxonomic_reranking_applied;
    let dynamic_headers: BTreeSet<String> = result
        .hits
        .iter()
        .filter_map(|hit| library.get(hit.library_index))
        .flat_map(|record| record.meta.headers.keys().cloned())
        .collect();

    let mut header = vec![
        "query_export_key".to_string(),
        "query_key_mode".to_string(),
        "hit_rank".to_string(),
        "hit_spectral_score".to_string(),
        "hit_matches".to_string(),
        "hit_precursor_mz".to_string(),
        "hit_ms1_deviation_ppm".to_string(),
        "hit_raw_name".to_string(),
    ];
    if include_taxonomy {
        header.splice(
            3..3,
            ["hit_rank_before_taxonomy".to_string()],
        );
        header.splice(
            5..5,
            [
                "hit_taxonomic_score".to_string(),
                "hit_combined_score".to_string(),
            ],
        );
        header.splice(
            7..7,
            [
                "hit_taxonomic_shared_rank".to_string(),
                "hit_taxonomic_organism_name".to_string(),
                "hit_taxonomic_organism_wikidata".to_string(),
                "hit_taxonomic_short_inchikey".to_string(),
            ],
        );
    }
    header.extend(dynamic_headers.iter().map(|key| format!("hit_{key}")));

    let mut out = String::new();
    push_tsv_row(&mut out, &header);

    for hit in &result.hits {
        let Some(query) = queries.get(hit.query_index) else {
            continue;
        };
        let Some(hit_record) = library.get(hit.library_index) else {
            continue;
        };

        let mut row = vec![
            query_key.value_for(query),
            query_key.label().to_string(),
            hit.rank.to_string(),
            format!("{:.8}", hit.spectral_score),
            hit.matches.to_string(),
            format!("{:.6}", hit_record.meta.precursor_mz),
            format!("{:.4}", hit.ms1_deviation_ppm),
            hit_record.meta.raw_name.clone(),
        ];
        if include_taxonomy {
            row.splice(
                3..3,
                [hit.rank_before_taxonomy.unwrap_or(hit.rank).to_string()],
            );
            row.splice(
                5..5,
                [
                    format!("{:.8}", hit.taxonomic_score),
                    format!("{:.8}", hit.combined_score),
                ],
            );
            row.splice(
                7..7,
                [
                    hit.matched_shared_rank.clone().unwrap_or_default(),
                    hit.matched_organism_name.clone().unwrap_or_default(),
                    hit.matched_organism_wikidata.clone().unwrap_or_default(),
                    hit.matched_short_inchikey.clone().unwrap_or_default(),
                ],
            );
        }
        row.extend(dynamic_headers.iter().map(|key| {
            hit_record
                .meta
                .headers
                .get(key)
                .cloned()
                .unwrap_or_default()
        }));
        push_tsv_row(&mut out, &row);
    }

    out
}

/// Renders a consensus artifact as a TSV table with one row per query spectrum.
pub fn export_consensus_tsv(
    result: &ConsensusArtifactResult,
    queries: &[SpectrumMetadata],
    query_key: SearchQueryKey,
    input_names: &[&str],
) -> String {
    let dynamic_headers: BTreeSet<String> = result
        .queries
        .iter()
        .filter_map(|row| row.annotation.as_ref())
        .flat_map(|annotation| annotation.representative_attributes.keys().cloned())
        .collect();
    let input_columns = input_names
        .iter()
        .map(|name| ((*name).to_string(), sanitize_column_name(name)))
        .collect::<Vec<_>>();

    let mut header = vec![
        "query_export_key".to_string(),
        "query_key_mode".to_string(),
        "annotation_present".to_string(),
        "consensus_key".to_string(),
        "consensus_score".to_string(),
        "consensus_class".to_string(),
        "support_count".to_string(),
        "support_hit_count".to_string(),
        "support_libraries".to_string(),
        "representative_input_name".to_string(),
        "representative_library_source_label".to_string(),
        "representative_rank".to_string(),
        "representative_rank_before_taxonomy".to_string(),
        "representative_spectral_score".to_string(),
        "representative_taxonomic_score".to_string(),
        "representative_combined_score".to_string(),
        "representative_matches".to_string(),
        "representative_precursor_mz".to_string(),
        "representative_ms1_deviation_ppm".to_string(),
        "representative_raw_name".to_string(),
        "representative_taxonomic_shared_rank".to_string(),
        "representative_taxonomic_organism_name".to_string(),
        "representative_taxonomic_organism_wikidata".to_string(),
        "representative_taxonomic_short_inchikey".to_string(),
        "representative_full_inchikey".to_string(),
        "exact_structure_consensus".to_string(),
    ];
    for (_, column) in &input_columns {
        header.extend([
            format!("best_rank_{column}"),
            format!("best_spectral_score_{column}"),
            format!("best_taxonomic_score_{column}"),
            format!("best_combined_score_{column}"),
            format!("best_matches_{column}"),
        ]);
    }
    header.extend(
        dynamic_headers
            .iter()
            .map(|key| format!("representative_{key}")),
    );

    let mut out = String::new();
    push_tsv_row(&mut out, &header);

    for row in &result.queries {
        let Some(query) = queries.get(row.query_index) else {
            continue;
        };
        let mut values = vec![
            query_key.value_for_meta(query),
            query_key.label().to_string(),
            row.annotation.is_some().to_string(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        ];
        if let Some(annotation) = row.annotation.as_ref() {
            values[3] = annotation.consensus_key.clone().unwrap_or_default();
            values[4] = format!("{:.8}", annotation.consensus_score);
            values[5] = consensus_class_label(annotation.consensus_class).to_string();
            values[6] = annotation.support_count.to_string();
            values[7] = annotation.support_hit_count.to_string();
            values[8] = annotation.support_libraries.join(";");
            values[9] = annotation.representative_input_name.clone();
            values[10] = annotation.representative_library_source_label.clone();
            values[11] = annotation.representative_rank.to_string();
            values[12] = annotation
                .representative_rank_before_taxonomy
                .map(|value| value.to_string())
                .unwrap_or_default();
            values[13] = format!("{:.8}", annotation.representative_spectral_score);
            values[14] = format!("{:.8}", annotation.representative_taxonomic_score);
            values[15] = format!("{:.8}", annotation.representative_combined_score);
            values[16] = annotation.representative_matches.to_string();
            values[17] = format!("{:.6}", annotation.representative_precursor_mz);
            values[18] = format!("{:.4}", annotation.representative_ms1_deviation_ppm);
            values[19] = annotation.representative_raw_name.clone();
            values[20] = annotation.representative_shared_rank.clone().unwrap_or_default();
            values[21] = annotation
                .representative_organism_name
                .clone()
                .unwrap_or_default();
            values[22] = annotation
                .representative_organism_wikidata
                .clone()
                .unwrap_or_default();
            values[23] = annotation
                .representative_short_inchikey
                .clone()
                .unwrap_or_default();
            values[24] = annotation
                .representative_full_inchikey
                .clone()
                .unwrap_or_default();
            values[25] = annotation.exact_structure_consensus.to_string();
        }

        for (input_name, _) in &input_columns {
            if let Some(annotation) = row.annotation.as_ref() {
                values.extend([
                    annotation
                        .best_rank_by_input
                        .get(input_name)
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                    annotation
                        .best_spectral_score_by_input
                        .get(input_name)
                        .map(|value| format!("{value:.8}"))
                        .unwrap_or_default(),
                    annotation
                        .best_taxonomic_score_by_input
                        .get(input_name)
                        .map(|value| format!("{value:.8}"))
                        .unwrap_or_default(),
                    annotation
                        .best_combined_score_by_input
                        .get(input_name)
                        .map(|value| format!("{value:.8}"))
                        .unwrap_or_default(),
                    annotation
                        .best_matches_by_input
                        .get(input_name)
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                ]);
            } else {
                values.extend([
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                ]);
            }
        }

        for key in &dynamic_headers {
            values.push(
                row.annotation
                    .as_ref()
                    .and_then(|annotation| annotation.representative_attributes.get(key))
                    .cloned()
                    .unwrap_or_default(),
            );
        }
        push_tsv_row(&mut out, &values);
    }

    out
}

#[derive(Serialize)]
struct JsonSearchExport<'a> {
    metric: &'a str,
    query_count: usize,
    library_count: usize,
    hit_count: usize,
    taxonomic_reranking_applied: bool,
    taxonomic_query: Option<&'a str>,
    rows: Vec<JsonSearchRow>,
}

#[derive(Serialize)]
struct JsonSearchRow {
    query_export_key: String,
    query_node_id: usize,
    query_feature_id: Option<String>,
    query_featurelist_feature_id: Option<String>,
    query_scans: Option<String>,
    query_label: String,
    query_raw_name: String,
    hit_rank: usize,
    hit_rank_before_taxonomy: Option<usize>,
    hit_spectral_score: f64,
    hit_taxonomic_score: f64,
    hit_combined_score: f64,
    hit_matches: usize,
    hit_taxonomic_shared_rank: Option<String>,
    hit_taxonomic_organism_name: Option<String>,
    hit_taxonomic_organism_wikidata: Option<String>,
    hit_taxonomic_short_inchikey: Option<String>,
    hit_precursor_mz: f64,
    hit_ms1_deviation_ppm: f64,
    hit_raw_name: String,
    hit_attributes: std::collections::BTreeMap<String, String>,
}

/// Renders a search artifact as a row-oriented JSON export.
pub fn export_search_json<TQ, TL>(
    result: &SearchArtifactResult,
    queries: &[SpectrumRecord<TQ>],
    library: &[SpectrumRecord<TL>],
    query_key: SearchQueryKey,
) -> Result<String, String> {
    let rows = result
        .hits
        .iter()
        .filter_map(|hit| {
            let query = queries.get(hit.query_index)?;
            let library_hit = library.get(hit.library_index)?;
            Some(JsonSearchRow {
                query_export_key: query_key.value_for(query),
                query_node_id: query.meta.id,
                query_feature_id: query.meta.feature_id.clone(),
                query_featurelist_feature_id: query.meta.featurelist_feature_id.clone(),
                query_scans: query.meta.scans.clone(),
                query_label: query.meta.label.clone(),
                query_raw_name: query.meta.raw_name.clone(),
                hit_rank: hit.rank,
                hit_rank_before_taxonomy: hit.rank_before_taxonomy,
                hit_spectral_score: hit.spectral_score,
                hit_taxonomic_score: hit.taxonomic_score,
                hit_combined_score: hit.combined_score,
                hit_matches: hit.matches,
                hit_taxonomic_shared_rank: hit.matched_shared_rank.clone(),
                hit_taxonomic_organism_name: hit.matched_organism_name.clone(),
                hit_taxonomic_organism_wikidata: hit.matched_organism_wikidata.clone(),
                hit_taxonomic_short_inchikey: hit.matched_short_inchikey.clone(),
                hit_precursor_mz: library_hit.meta.precursor_mz,
                hit_ms1_deviation_ppm: hit.ms1_deviation_ppm,
                hit_raw_name: library_hit.meta.raw_name.clone(),
                hit_attributes: library_hit.meta.headers.clone(),
            })
        })
        .collect::<Vec<_>>();

    serde_json::to_string_pretty(&JsonSearchExport {
        metric: result.metric.label(),
        query_count: result.query_count,
        library_count: result.library_count,
        hit_count: rows.len(),
        taxonomic_reranking_applied: result.taxonomic_reranking_applied,
        taxonomic_query: result.taxonomic_query.as_deref(),
        rows,
    })
    .map_err(|err| format!("failed to serialize JSON export: {err}"))
}

/// Writes TSV contents to disk, creating parent directories when needed.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_tsv_to_path(path: &std::path::Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    std::fs::write(path, contents)
        .map_err(|err| format!("failed to write {}: {err}", path.display()))
}

/// Writes JSON contents to disk, creating parent directories when needed.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_json_to_path(path: &std::path::Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    std::fs::write(path, contents)
        .map_err(|err| format!("failed to write {}: {err}", path.display()))
}

/// Downloads a generated TSV file in the browser.
#[cfg(target_arch = "wasm32")]
pub fn download_tsv_file(filename: &str, contents: &str) -> Result<(), String> {
    download_one(
        filename,
        "text/tab-separated-values;charset=utf-8",
        contents.as_bytes(),
    )
}

#[cfg(target_arch = "wasm32")]
fn download_one(filename: &str, mime_type: &str, bytes: &[u8]) -> Result<(), String> {
    use wasm_bindgen::JsCast;

    let window = web_sys::window().ok_or("window unavailable")?;
    let document = window.document().ok_or("document unavailable")?;

    let array = js_sys::Uint8Array::from(bytes);
    let array_parts = js_sys::Array::new();
    array_parts.push(&array.buffer());

    let bag = web_sys::BlobPropertyBag::new();
    bag.set_type(mime_type);
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&array_parts, &bag)
        .map_err(|err| format!("failed to build Blob: {err:?}"))?;

    let url = web_sys::Url::create_object_url_with_blob(&blob)
        .map_err(|err| format!("failed to create object URL: {err:?}"))?;

    let anchor = document
        .create_element("a")
        .map_err(|err| format!("failed to create anchor: {err:?}"))?
        .dyn_into::<web_sys::HtmlAnchorElement>()
        .map_err(|_| "failed to cast anchor element".to_string())?;
    anchor.set_href(&url);
    anchor.set_download(filename);
    anchor.click();

    web_sys::Url::revoke_object_url(&url)
        .map_err(|err| format!("failed to revoke object URL: {err:?}"))?;
    Ok(())
}

fn escape_tsv(value: &str) -> String {
    value
        .replace('\t', " ")
        .replace('\n', " ")
        .replace('\r', " ")
}

fn push_tsv_row(out: &mut String, values: &[String]) {
    let mut first = true;
    for value in values {
        if !first {
            out.push('\t');
        }
        first = false;
        out.push_str(&escape_tsv(value));
    }
    out.push('\n');
}

fn sanitize_column_name(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        "input".to_string()
    } else {
        trimmed.to_string()
    }
}

fn consensus_class_label(value: ConsensusClass) -> &'static str {
    match value {
        ConsensusClass::Singleton => "singleton",
        ConsensusClass::CrossLibraryExact => "cross_library_exact",
        ConsensusClass::CrossLibraryShortInchikey => "cross_library_short_inchikey",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use mass_spectrometry::prelude::{GenericSpectrum, SpectrumAlloc};

    use crate::api::{SearchArtifactHit, SearchArtifactResult};
    use crate::model::{SpectrumMetadata, SpectrumRecord};

    use super::{SearchQueryKey, export_search_json, export_search_tsv};

    fn spectrum_record(id: usize, raw_name: &str, headers: &[(&str, &str)]) -> SpectrumRecord {
        let spectrum = GenericSpectrum::with_capacity(100.0 + id as f64, 0).expect("spectrum");
        let mut header_map = BTreeMap::new();
        for (key, value) in headers {
            header_map.insert((*key).to_string(), (*value).to_string());
        }
        SpectrumRecord {
            meta: SpectrumMetadata {
                id,
                label: format!("label_{id}"),
                raw_name: raw_name.to_string(),
                feature_id: Some(format!("feature_{id}")),
                scans: Some(format!("scan_{id}")),
                filename: None,
                source_scan_usi: None,
                featurelist_feature_id: Some(format!("flf_{id}")),
                headers: header_map,
                precursor_mz: 100.0 + id as f64,
                num_peaks: 0,
            },
            peaks: Arc::new(Vec::new()),
            spectrum: Arc::new(spectrum),
            payload: (),
        }
    }

    #[test]
    fn search_tsv_exports_one_row_per_hit_with_dynamic_headers() {
        let queries = vec![spectrum_record(0, "query 0", &[("QUERY_ONLY", "x")])];
        let library = vec![
            spectrum_record(
                10,
                "hit one",
                &[("COMPOUND_NAME", "cmpd\t1"), ("INCHIKEY", "AAAA")],
            ),
            spectrum_record(11, "hit two", &[("SMILES", "C\nC"), ("ADDUCT", "[M+H]+")]),
        ];
        let result = SearchArtifactResult {
            hits: vec![
                SearchArtifactHit {
                    query_index: 0,
                    library_index: 0,
                    rank: 1,
                    rank_before_taxonomy: None,
                    spectral_score: 0.95,
                    ms1_deviation_ppm: 100_000.0,
                    taxonomic_score: 0.0,
                    combined_score: 0.95,
                    matches: 6,
                    matched_organism_name: None,
                    matched_organism_wikidata: None,
                    matched_shared_rank: None,
                    matched_short_inchikey: None,
                },
                SearchArtifactHit {
                    query_index: 0,
                    library_index: 1,
                    rank: 2,
                    rank_before_taxonomy: None,
                    spectral_score: 0.75,
                    ms1_deviation_ppm: 110_000.0,
                    taxonomic_score: 0.0,
                    combined_score: 0.75,
                    matches: 4,
                    matched_organism_name: None,
                    matched_organism_wikidata: None,
                    matched_shared_rank: None,
                    matched_short_inchikey: None,
                },
            ],
            query_count: 1,
            library_count: 2,
            metric: crate::similarity::SimilarityMetric::LinearCosine,
            taxonomic_reranking_applied: false,
            taxonomic_query: None,
        };

        let tsv = export_search_tsv(&result, &queries, &library, SearchQueryKey::FeatureId);
        let mut lines = tsv.lines();
        let header = lines.next().expect("header");
        assert!(!header.contains("hit_taxonomic_score"));
        assert!(!header.contains("hit_combined_score"));
        assert!(!header.contains("hit_taxonomic_shared_rank"));
        assert!(header.contains("hit_COMPOUND_NAME"));
        assert!(header.contains("hit_INCHIKEY"));
        assert!(header.contains("hit_SMILES"));
        assert!(header.contains("hit_ADDUCT"));
        assert!(header.contains("query_key_mode"));
        assert!(header.contains("hit_spectral_score"));
        let first = lines.next().expect("first data row");
        let columns: Vec<_> = first.split('\t').collect();
        assert_eq!(columns[0], "feature_0");
        assert_eq!(columns[1], "FEATURE_ID");
        assert_eq!(columns[2], "1");
        assert_eq!(columns[3], "0.95000000");
        assert_eq!(columns[4], "6");
        assert_eq!(columns[5], "110.000000");
        assert_eq!(columns[6], "100000.0000");
        assert_eq!(columns[7], "hit one");
    }

    #[test]
    fn search_tsv_includes_taxonomy_columns_when_reranking_is_applied() {
        let queries = vec![spectrum_record(0, "query 0", &[])];
        let library = vec![spectrum_record(10, "hit one", &[("INCHIKEY", "AAAA")])];
        let result = SearchArtifactResult {
            hits: vec![SearchArtifactHit {
                query_index: 0,
                library_index: 0,
                rank: 1,
                rank_before_taxonomy: Some(2),
                spectral_score: 0.95,
                ms1_deviation_ppm: 100_000.0,
                taxonomic_score: 2.0,
                combined_score: 2.95,
                matches: 6,
                matched_organism_name: Some("Plantus exampleus".to_string()),
                matched_organism_wikidata: Some("Q123".to_string()),
                matched_shared_rank: Some("genus".to_string()),
                matched_short_inchikey: Some("AAAA".to_string()),
            }],
            query_count: 1,
            library_count: 1,
            metric: crate::similarity::SimilarityMetric::LinearCosine,
            taxonomic_reranking_applied: true,
            taxonomic_query: Some("Example taxon".to_string()),
        };

        let tsv = export_search_tsv(&result, &queries, &library, SearchQueryKey::FeatureId);
        let mut lines = tsv.lines();
        let header = lines.next().expect("header");
        assert!(header.contains("hit_taxonomic_score"));
        assert!(header.contains("hit_combined_score"));
        assert!(header.contains("hit_taxonomic_shared_rank"));
        let first = lines.next().expect("first data row");
        let columns: Vec<_> = first.split('\t').collect();
        assert_eq!(columns[3], "2");
        assert_eq!(columns[5], "2.00000000");
        assert_eq!(columns[6], "2.95000000");
        assert_eq!(columns[7], "genus");
        assert_eq!(columns[8], "Plantus exampleus");
        assert_eq!(columns[9], "Q123");
        assert_eq!(columns[10], "AAAA");
    }

    #[test]
    fn search_json_exports_rows() {
        let queries = vec![spectrum_record(0, "query 0", &[])];
        let library = vec![spectrum_record(10, "hit one", &[("INCHIKEY", "AAAA")])];
        let result = SearchArtifactResult {
            hits: vec![SearchArtifactHit {
                query_index: 0,
                library_index: 0,
                rank: 1,
                rank_before_taxonomy: None,
                spectral_score: 0.95,
                ms1_deviation_ppm: 100_000.0,
                taxonomic_score: 0.0,
                combined_score: 0.95,
                matches: 6,
                matched_organism_name: None,
                matched_organism_wikidata: None,
                matched_shared_rank: None,
                matched_short_inchikey: None,
            }],
            query_count: 1,
            library_count: 1,
            metric: crate::similarity::SimilarityMetric::LinearCosine,
            taxonomic_reranking_applied: false,
            taxonomic_query: None,
        };

        let json = export_search_json(&result, &queries, &library, SearchQueryKey::FeatureId)
            .expect("json export");
        assert!(json.contains("\"metric\": \"LinearCosine\""));
        assert!(json.contains("\"hit_raw_name\": \"hit one\""));
        assert!(json.contains("\"hit_ms1_deviation_ppm\": 100000.0"));
        assert!(json.contains("\"INCHIKEY\": \"AAAA\""));
        assert!(json.contains("\"hit_rank_before_taxonomy\": null"));
    }
}
