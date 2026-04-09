//! Cross-library consensus fusion for one-hit-per-query display workflows.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::api::{
    ConsensusAnnotation, ConsensusArtifact, ConsensusArtifactResult, ConsensusClass,
    ConsensusInputSummary, ConsensusMergeParams, ConsensusQueryResult, ConsensusSupportHit,
    SearchArtifact,
};
use crate::export::{SearchQueryKey, export_consensus_tsv};
use crate::model::SpectrumMetadata;
use crate::taxonomy::short_inchikey;

/// Merges two search artifacts into one query-oriented consensus artifact.
pub fn merge_search_artifacts(
    left_name: &str,
    left: SearchArtifact,
    right_name: &str,
    right: SearchArtifact,
    merge: ConsensusMergeParams,
    query_key: Option<SearchQueryKey>,
) -> Result<ConsensusArtifact, String> {
    validate_merge_inputs(&left, &right)?;
    validate_merge_params(&merge)?;

    let query_key = query_key.unwrap_or(left.query_key);
    let queries = left.query_spectra.clone();
    let inputs = [
        MergeInput {
            index: 0,
            name: left_name.to_string(),
            artifact: &left,
            weight: merge.left_weight,
        },
        MergeInput {
            index: 1,
            name: right_name.to_string(),
            artifact: &right,
            weight: merge.right_weight,
        },
    ];

    let rows = (0..queries.len())
        .map(|query_index| ConsensusQueryResult {
            query_index,
            annotation: merge_query_hits(query_index, &inputs, &merge),
        })
        .collect::<Vec<_>>();
    let annotated_query_count = rows.iter().filter(|row| row.annotation.is_some()).count();
    let result = ConsensusArtifactResult {
        queries: rows,
        query_count: queries.len(),
        annotated_query_count,
    };
    let summaries = inputs
        .iter()
        .map(|input| ConsensusInputSummary {
            name: input.name.clone(),
            query_source_label: input.artifact.query_source_label.clone(),
            library_source_label: input.artifact.library_source_label.clone(),
            search: input.artifact.search.clone(),
            metric: input.artifact.result.metric,
            taxonomic_reranking_applied: input.artifact.result.taxonomic_reranking_applied,
            taxonomic_query: input.artifact.result.taxonomic_query.clone(),
        })
        .collect::<Vec<_>>();
    let tsv = export_consensus_tsv(&result, &queries, query_key, &[left_name, right_name]);

    Ok(ConsensusArtifact {
        query_source_label: left.query_source_label,
        query_key,
        query_spectra: queries,
        merge,
        inputs: summaries,
        result,
        tsv,
    })
}

struct MergeInput<'a> {
    index: usize,
    name: String,
    artifact: &'a SearchArtifact,
    weight: f64,
}

#[derive(Clone)]
struct SupportCandidate {
    input_index: usize,
    input_name: String,
    library_source_label: String,
    library_index: usize,
    rank: usize,
    rank_before_taxonomy: Option<usize>,
    spectral_score: f64,
    taxonomic_score: f64,
    combined_score: f64,
    matches: usize,
    precursor_mz: f64,
    ms1_deviation_ppm: f64,
    raw_name: String,
    organism_name: Option<String>,
    organism_wikidata: Option<String>,
    shared_rank: Option<String>,
    short_inchikey: Option<String>,
    full_inchikey: Option<String>,
    attributes: BTreeMap<String, String>,
}

struct GroupAccumulator {
    consensus_key: Option<String>,
    support_hit_count: usize,
    full_inchikeys: BTreeSet<String>,
    best_by_input: HashMap<usize, SupportCandidate>,
}

impl GroupAccumulator {
    fn new(consensus_key: Option<String>) -> Self {
        Self {
            consensus_key,
            support_hit_count: 0,
            full_inchikeys: BTreeSet::new(),
            best_by_input: HashMap::new(),
        }
    }

    fn insert(&mut self, candidate: SupportCandidate) {
        self.support_hit_count += 1;
        if let Some(full) = candidate.full_inchikey.clone() {
            self.full_inchikeys.insert(full);
        }
        match self.best_by_input.get(&candidate.input_index) {
            Some(existing) if !support_candidate_better(&candidate, existing) => {}
            _ => {
                self.best_by_input.insert(candidate.input_index, candidate);
            }
        }
    }
}

fn validate_merge_inputs(left: &SearchArtifact, right: &SearchArtifact) -> Result<(), String> {
    if left.query_source_label != right.query_source_label {
        return Err(format!(
            "query source labels differ: '{}' vs '{}'",
            left.query_source_label, right.query_source_label
        ));
    }
    if left.query_spectra != right.query_spectra {
        return Err("query spectra differ between input artifacts".to_string());
    }
    Ok(())
}

fn validate_merge_params(params: &ConsensusMergeParams) -> Result<(), String> {
    if params.top_k_per_library == 0 {
        return Err("top_k_per_library must be at least 1".to_string());
    }
    if params.rrf_k <= 0.0 {
        return Err("rrf_k must be positive".to_string());
    }
    if params.left_weight <= 0.0 || params.right_weight <= 0.0 {
        return Err("consensus input weights must be positive".to_string());
    }
    Ok(())
}

fn merge_query_hits(
    query_index: usize,
    inputs: &[MergeInput<'_>],
    params: &ConsensusMergeParams,
) -> Option<ConsensusAnnotation> {
    let mut groups: HashMap<String, GroupAccumulator> = HashMap::new();

    for input in inputs {
        for hit in input
            .artifact
            .result
            .hits
            .iter()
            .filter(|hit| hit.query_index == query_index && hit.rank <= params.top_k_per_library)
        {
            let Some(library_meta) = input.artifact.library_spectra.get(hit.library_index) else {
                continue;
            };
            let full_inchikey = full_inchikey_from_metadata(library_meta);
            let short_key = hit
                .matched_short_inchikey
                .clone()
                .or_else(|| full_inchikey.as_deref().and_then(short_inchikey))
                .or_else(|| short_inchikey_from_metadata(library_meta));
            let group_key = short_key
                .as_ref()
                .map(|value| format!("short:{value}"))
                .unwrap_or_else(|| format!("unique:{}:{}", input.index, hit.library_index));
            let candidate = SupportCandidate {
                input_index: input.index,
                input_name: input.name.clone(),
                library_source_label: input.artifact.library_source_label.clone(),
                library_index: hit.library_index,
                rank: hit.rank,
                rank_before_taxonomy: hit.rank_before_taxonomy,
                spectral_score: hit.spectral_score,
                taxonomic_score: hit.taxonomic_score,
                combined_score: hit.combined_score,
                matches: hit.matches,
                precursor_mz: library_meta.precursor_mz,
                ms1_deviation_ppm: hit.ms1_deviation_ppm,
                raw_name: library_meta.raw_name.clone(),
                organism_name: hit.matched_organism_name.clone(),
                organism_wikidata: hit.matched_organism_wikidata.clone(),
                shared_rank: hit.matched_shared_rank.clone(),
                short_inchikey: short_key.clone(),
                full_inchikey,
                attributes: library_meta.headers.clone(),
            };
            groups
                .entry(group_key)
                .or_insert_with(|| GroupAccumulator::new(short_key))
                .insert(candidate);
        }
    }

    let mut annotations = groups
        .into_values()
        .filter_map(|group| finalize_group(group, inputs, params))
        .collect::<Vec<_>>();
    annotations.sort_by(consensus_annotation_order);
    annotations.into_iter().next()
}

fn finalize_group(
    group: GroupAccumulator,
    inputs: &[MergeInput<'_>],
    params: &ConsensusMergeParams,
) -> Option<ConsensusAnnotation> {
    let mut support_hits = inputs
        .iter()
        .filter_map(|input| group.best_by_input.get(&input.index).cloned())
        .collect::<Vec<_>>();
    if support_hits.is_empty() {
        return None;
    }

    support_hits.sort_by(|left, right| left.input_index.cmp(&right.input_index));
    let support_count = support_hits.len();
    let support_libraries = support_hits
        .iter()
        .map(|hit| hit.input_name.clone())
        .collect::<Vec<_>>();

    let mut consensus_score = 0.0;
    let mut best_rank_by_input = BTreeMap::new();
    let mut best_spectral_score_by_input = BTreeMap::new();
    let mut best_taxonomic_score_by_input = BTreeMap::new();
    let mut best_combined_score_by_input = BTreeMap::new();
    let mut best_matches_by_input = BTreeMap::new();
    for hit in &support_hits {
        let weight = inputs
            .iter()
            .find(|input| input.index == hit.input_index)
            .map(|input| input.weight)
            .unwrap_or(1.0);
        consensus_score += weight / (params.rrf_k + hit.rank as f64);
        best_rank_by_input.insert(hit.input_name.clone(), hit.rank);
        best_spectral_score_by_input.insert(hit.input_name.clone(), hit.spectral_score);
        best_taxonomic_score_by_input.insert(hit.input_name.clone(), hit.taxonomic_score);
        best_combined_score_by_input.insert(hit.input_name.clone(), hit.combined_score);
        best_matches_by_input.insert(hit.input_name.clone(), hit.matches);
    }
    if support_count > 1 {
        consensus_score += params.consensus_bonus;
    }

    let representative = support_hits
        .iter()
        .min_by(|left, right| support_candidate_order(left, right))
        .cloned()?;
    let exact_structure_consensus = support_count > 1 && group.full_inchikeys.len() == 1;
    let consensus_class = if support_count <= 1 {
        ConsensusClass::Singleton
    } else if exact_structure_consensus {
        ConsensusClass::CrossLibraryExact
    } else {
        ConsensusClass::CrossLibraryShortInchikey
    };

    Some(ConsensusAnnotation {
        consensus_key: group.consensus_key,
        consensus_score,
        consensus_class,
        exact_structure_consensus,
        support_libraries,
        support_count,
        support_hit_count: group.support_hit_count,
        best_rank_by_input,
        best_spectral_score_by_input,
        best_taxonomic_score_by_input,
        best_combined_score_by_input,
        best_matches_by_input,
        representative_input_name: representative.input_name.clone(),
        representative_library_source_label: representative.library_source_label.clone(),
        representative_library_index: representative.library_index,
        representative_rank: representative.rank,
        representative_rank_before_taxonomy: representative.rank_before_taxonomy,
        representative_spectral_score: representative.spectral_score,
        representative_taxonomic_score: representative.taxonomic_score,
        representative_combined_score: representative.combined_score,
        representative_matches: representative.matches,
        representative_precursor_mz: representative.precursor_mz,
        representative_ms1_deviation_ppm: representative.ms1_deviation_ppm,
        representative_raw_name: representative.raw_name.clone(),
        representative_organism_name: representative.organism_name.clone(),
        representative_organism_wikidata: representative.organism_wikidata.clone(),
        representative_shared_rank: representative.shared_rank.clone(),
        representative_short_inchikey: representative.short_inchikey.clone(),
        representative_full_inchikey: representative.full_inchikey.clone(),
        representative_attributes: representative.attributes.clone(),
        support_hits: support_hits
            .into_iter()
            .map(|hit| ConsensusSupportHit {
                input_name: hit.input_name,
                library_source_label: hit.library_source_label,
                library_index: hit.library_index,
                rank: hit.rank,
                rank_before_taxonomy: hit.rank_before_taxonomy,
                spectral_score: hit.spectral_score,
                taxonomic_score: hit.taxonomic_score,
                combined_score: hit.combined_score,
                matches: hit.matches,
                precursor_mz: hit.precursor_mz,
                ms1_deviation_ppm: hit.ms1_deviation_ppm,
                raw_name: hit.raw_name,
                organism_name: hit.organism_name,
                organism_wikidata: hit.organism_wikidata,
                shared_rank: hit.shared_rank,
                short_inchikey: hit.short_inchikey,
                full_inchikey: hit.full_inchikey,
                attributes: hit.attributes,
            })
            .collect(),
    })
}

fn support_candidate_better(candidate: &SupportCandidate, existing: &SupportCandidate) -> bool {
    support_candidate_order(candidate, existing).is_lt()
}

fn support_candidate_order(
    left: &SupportCandidate,
    right: &SupportCandidate,
) -> std::cmp::Ordering {
    left.rank
        .cmp(&right.rank)
        .then_with(|| right.combined_score.total_cmp(&left.combined_score))
        .then_with(|| right.spectral_score.total_cmp(&left.spectral_score))
        .then_with(|| right.matches.cmp(&left.matches))
        .then_with(|| left.library_index.cmp(&right.library_index))
}

fn consensus_annotation_order(
    left: &ConsensusAnnotation,
    right: &ConsensusAnnotation,
) -> std::cmp::Ordering {
    right
        .consensus_score
        .total_cmp(&left.consensus_score)
        .then_with(|| right.support_count.cmp(&left.support_count))
        .then_with(|| left.representative_rank.cmp(&right.representative_rank))
        .then_with(|| {
            right
                .representative_combined_score
                .total_cmp(&left.representative_combined_score)
        })
        .then_with(|| {
            right
                .representative_spectral_score
                .total_cmp(&left.representative_spectral_score)
        })
        .then_with(|| left.representative_input_name.cmp(&right.representative_input_name))
}

fn normalize_header_key(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

fn full_inchikey_from_metadata(meta: &SpectrumMetadata) -> Option<String> {
    let mut best: Option<(usize, String)> = None;
    for (key, value) in &meta.headers {
        let normalized = normalize_header_key(key);
        let priority = if normalized == "ik2d" {
            0
        } else if normalized == "structureinchikey" || normalized == "gnpsinchikey" {
            1
        } else if normalized.contains("inchikey") {
            2
        } else {
            continue;
        };
        let candidate = value.trim().trim_matches('"').to_ascii_uppercase();
        if candidate.len() < 14 {
            continue;
        }
        match &best {
            Some((best_priority, _)) if *best_priority <= priority => {}
            _ => best = Some((priority, candidate)),
        }
    }
    best.map(|(_, value)| value)
}

fn short_inchikey_from_metadata(meta: &SpectrumMetadata) -> Option<String> {
    full_inchikey_from_metadata(meta)
        .as_deref()
        .and_then(short_inchikey)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::api::{ConsensusClass, SearchArtifactHit, SearchArtifactResult};
    use crate::model::{ParseStats, SpectrumMetadata};
    use crate::search::LibrarySearchParams;
    use crate::similarity::{ComputeParams, SimilarityMetric};

    use super::*;

    fn query_meta(id: usize) -> SpectrumMetadata {
        SpectrumMetadata {
            id,
            label: format!("query_{id}"),
            raw_name: format!("query_{id}"),
            feature_id: Some(format!("feature_{id}")),
            scans: Some(format!("scan_{id}")),
            filename: None,
            source_scan_usi: None,
            featurelist_feature_id: None,
            headers: BTreeMap::new(),
            precursor_mz: 100.0 + id as f64,
            num_peaks: 3,
        }
    }

    fn library_meta(id: usize, raw_name: &str, inchikey: Option<&str>) -> SpectrumMetadata {
        let mut headers = BTreeMap::new();
        if let Some(inchikey) = inchikey {
            headers.insert("INCHIKEY".to_string(), inchikey.to_string());
        }
        SpectrumMetadata {
            id,
            label: raw_name.to_string(),
            raw_name: raw_name.to_string(),
            feature_id: None,
            scans: None,
            filename: None,
            source_scan_usi: None,
            featurelist_feature_id: None,
            headers,
            precursor_mz: 200.0 + id as f64,
            num_peaks: 5,
        }
    }

    fn hit(
        query_index: usize,
        library_index: usize,
        rank: usize,
        combined_score: f64,
        short_inchikey: Option<&str>,
    ) -> SearchArtifactHit {
        SearchArtifactHit {
            query_index,
            library_index,
            rank,
            rank_before_taxonomy: Some(rank + 1),
            spectral_score: combined_score - 1.0,
            ms1_deviation_ppm: rank as f64,
            taxonomic_score: 1.0,
            combined_score,
            matches: 6,
            matched_organism_name: Some("Withania somnifera".to_string()),
            matched_organism_wikidata: Some("Q1".to_string()),
            matched_shared_rank: Some("species".to_string()),
            matched_short_inchikey: short_inchikey.map(ToOwned::to_owned),
        }
    }

    fn artifact(
        library_source_label: &str,
        query_spectra: Vec<SpectrumMetadata>,
        library_spectra: Vec<SpectrumMetadata>,
        hits: Vec<SearchArtifactHit>,
    ) -> SearchArtifact {
        SearchArtifact {
            query_source_label: "query.mgf".to_string(),
            library_source_label: library_source_label.to_string(),
            query_stats: ParseStats::default(),
            library_stats: ParseStats::default(),
            search: LibrarySearchParams {
                compute: ComputeParams {
                    metric: SimilarityMetric::LinearCosine,
                    fragment_mz_tolerance: 0.2,
                    mz_power: 0.0,
                    intensity_power: 1.0,
                    top_n_peaks: None,
                },
                precursor_mz_tolerance: 0.01,
                min_matched_peaks: 3,
                min_similarity_threshold: 0.7,
                top_n: 20,
            },
            taxonomy: Some(crate::api::SearchTaxonomyRequest {
                query_text: "Withania somnifera".to_string(),
                lotus_source_label: "lotus.csv".to_string(),
                lotus_csv_text: None,
                lotus_csv_path: None,
            }),
            query_key: SearchQueryKey::FeatureId,
            query_spectra,
            library_spectra,
            result: SearchArtifactResult {
                hits,
                query_count: 2,
                library_count: 3,
                metric: SimilarityMetric::LinearCosine,
                taxonomic_reranking_applied: true,
                taxonomic_query: Some("Withania somnifera".to_string()),
            },
            tsv: String::new(),
        }
    }

    #[test]
    fn cross_library_consensus_can_beat_singleton_top_hit() {
        let queries = vec![query_meta(0), query_meta(1)];
        let left = artifact(
            "gnps.mgf",
            queries.clone(),
            vec![
                library_meta(0, "singleton", Some("BBBBBBBBBBBBBB-AAAA")),
                library_meta(1, "consensus left", Some("AAAAAAAAAAAAAA-BBBB")),
            ],
            vec![hit(0, 0, 1, 1.9, Some("BBBBBBBBBBBBBB")), hit(0, 1, 3, 1.7, Some("AAAAAAAAAAAAAA"))],
        );
        let right = artifact(
            "isdb.mgf",
            queries.clone(),
            vec![
                library_meta(0, "other singleton", Some("CCCCCCCCCCCCCC-DDDD")),
                library_meta(1, "consensus right", Some("AAAAAAAAAAAAAA-CCCC")),
            ],
            vec![hit(0, 0, 1, 1.8, Some("CCCCCCCCCCCCCC")), hit(0, 1, 2, 1.75, Some("AAAAAAAAAAAAAA"))],
        );

        let merged = merge_search_artifacts("gnps", left, "isdb", right, ConsensusMergeParams::default(), None)
            .expect("merge");
        let annotation = merged.result.queries[0].annotation.as_ref().expect("annotation");
        assert_eq!(annotation.consensus_key.as_deref(), Some("AAAAAAAAAAAAAA"));
        assert_eq!(annotation.support_count, 2);
        assert_eq!(annotation.support_libraries, vec!["gnps", "isdb"]);
    }

    #[test]
    fn out_of_window_consensus_does_not_override_singleton() {
        let queries = vec![query_meta(0), query_meta(1)];
        let left = artifact(
            "gnps.mgf",
            queries.clone(),
            vec![
                library_meta(0, "singleton", Some("BBBBBBBBBBBBBB-AAAA")),
                library_meta(1, "deep consensus", Some("AAAAAAAAAAAAAA-BBBB")),
            ],
            vec![hit(0, 0, 1, 2.0, Some("BBBBBBBBBBBBBB")), hit(0, 1, 6, 1.6, Some("AAAAAAAAAAAAAA"))],
        );
        let right = artifact(
            "isdb.mgf",
            queries.clone(),
            vec![library_meta(0, "consensus right", Some("AAAAAAAAAAAAAA-CCCC"))],
            vec![hit(0, 0, 2, 1.7, Some("AAAAAAAAAAAAAA"))],
        );

        let merged = merge_search_artifacts("gnps", left, "isdb", right, ConsensusMergeParams::default(), None)
            .expect("merge");
        let annotation = merged.result.queries[0].annotation.as_ref().expect("annotation");
        assert_eq!(annotation.consensus_key.as_deref(), Some("BBBBBBBBBBBBBB"));
        assert_eq!(annotation.consensus_class, ConsensusClass::Singleton);
    }

    #[test]
    fn singleton_is_retained_when_only_one_library_supports_query() {
        let queries = vec![query_meta(0), query_meta(1)];
        let left = artifact(
            "gnps.mgf",
            queries.clone(),
            vec![library_meta(0, "singleton", Some("BBBBBBBBBBBBBB-AAAA"))],
            vec![hit(0, 0, 1, 2.0, Some("BBBBBBBBBBBBBB"))],
        );
        let right = artifact("isdb.mgf", queries.clone(), Vec::new(), Vec::new());

        let merged = merge_search_artifacts("gnps", left, "isdb", right, ConsensusMergeParams::default(), None)
            .expect("merge");
        let annotation = merged.result.queries[0].annotation.as_ref().expect("annotation");
        assert_eq!(annotation.support_count, 1);
        assert_eq!(annotation.support_libraries, vec!["gnps"]);
    }

    #[test]
    fn duplicate_hits_in_one_library_are_collapsed_before_scoring() {
        let queries = vec![query_meta(0), query_meta(1)];
        let left = artifact(
            "gnps.mgf",
            queries.clone(),
            vec![
                library_meta(0, "consensus a", Some("AAAAAAAAAAAAAA-BBBB")),
                library_meta(1, "consensus b", Some("AAAAAAAAAAAAAA-CCCC")),
            ],
            vec![hit(0, 0, 2, 1.8, Some("AAAAAAAAAAAAAA")), hit(0, 1, 3, 1.7, Some("AAAAAAAAAAAAAA"))],
        );
        let right = artifact(
            "isdb.mgf",
            queries.clone(),
            vec![library_meta(0, "consensus right", Some("AAAAAAAAAAAAAA-DDDD"))],
            vec![hit(0, 0, 1, 1.9, Some("AAAAAAAAAAAAAA"))],
        );

        let merged = merge_search_artifacts("gnps", left, "isdb", right, ConsensusMergeParams::default(), None)
            .expect("merge");
        let annotation = merged.result.queries[0].annotation.as_ref().expect("annotation");
        assert_eq!(annotation.support_hit_count, 3);
        assert_eq!(annotation.support_hits.len(), 2);
        assert_eq!(annotation.best_rank_by_input["gnps"], 2);
    }

    #[test]
    fn mixed_full_structures_are_marked_as_short_inchikey_consensus() {
        let queries = vec![query_meta(0), query_meta(1)];
        let left = artifact(
            "gnps.mgf",
            queries.clone(),
            vec![library_meta(0, "left", Some("AAAAAAAAAAAAAA-BBBB"))],
            vec![hit(0, 0, 1, 1.8, Some("AAAAAAAAAAAAAA"))],
        );
        let right = artifact(
            "isdb.mgf",
            queries.clone(),
            vec![library_meta(0, "right", Some("AAAAAAAAAAAAAA-CCCC"))],
            vec![hit(0, 0, 2, 1.7, Some("AAAAAAAAAAAAAA"))],
        );

        let merged = merge_search_artifacts("gnps", left, "isdb", right, ConsensusMergeParams::default(), None)
            .expect("merge");
        let annotation = merged.result.queries[0].annotation.as_ref().expect("annotation");
        assert_eq!(annotation.consensus_class, ConsensusClass::CrossLibraryShortInchikey);
        assert!(!annotation.exact_structure_consensus);
    }

    #[test]
    fn query_order_is_preserved_and_empty_queries_are_kept() {
        let queries = vec![query_meta(0), query_meta(1)];
        let left = artifact(
            "gnps.mgf",
            queries.clone(),
            vec![library_meta(0, "left", Some("AAAAAAAAAAAAAA-BBBB"))],
            vec![hit(0, 0, 1, 1.8, Some("AAAAAAAAAAAAAA"))],
        );
        let right = artifact("isdb.mgf", queries.clone(), Vec::new(), Vec::new());

        let merged = merge_search_artifacts("gnps", left, "isdb", right, ConsensusMergeParams::default(), None)
            .expect("merge");
        assert_eq!(merged.result.queries.len(), 2);
        assert_eq!(merged.result.queries[0].query_index, 0);
        assert_eq!(merged.result.queries[1].query_index, 1);
        assert!(merged.result.queries[0].annotation.is_some());
        assert!(merged.result.queries[1].annotation.is_none());
    }
}
