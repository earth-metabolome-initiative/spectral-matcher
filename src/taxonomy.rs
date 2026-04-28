//! LOTUS taxonomy loading and lineage-matching helpers used for search reranking.

use std::collections::HashMap;
use std::io::Read;

use crate::model::SpectrumRecord;

#[cfg(not(target_arch = "wasm32"))]
const OTOL_TNRS_MATCH_NAMES_URL: &str = "https://api.opentreeoflife.org/v3/tnrs/match_names";

const TAXONOMY_COLUMN_NAMES: [&str; 10] = [
    "organism_taxonomy_01domain",
    "organism_taxonomy_02kingdom",
    "organism_taxonomy_03phylum",
    "organism_taxonomy_04class",
    "organism_taxonomy_05order",
    "organism_taxonomy_06family",
    "organism_taxonomy_07tribe",
    "organism_taxonomy_08genus",
    "organism_taxonomy_09species",
    "organism_taxonomy_10varietas",
];

/// Ordered taxonomy ranks supported by the reranking workflow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TaxonomicRank {
    Domain,
    Kingdom,
    Phylum,
    Class,
    Order,
    Family,
    Tribe,
    Genus,
    Species,
    Varietas,
}

impl TaxonomicRank {
    /// All supported taxonomy ranks from broadest to most specific.
    pub const ALL: [Self; 10] = [
        Self::Domain,
        Self::Kingdom,
        Self::Phylum,
        Self::Class,
        Self::Order,
        Self::Family,
        Self::Tribe,
        Self::Genus,
        Self::Species,
        Self::Varietas,
    ];

    /// Human-readable lowercase label for the rank.
    pub fn label(self) -> &'static str {
        match self {
            Self::Domain => "domain",
            Self::Kingdom => "kingdom",
            Self::Phylum => "phylum",
            Self::Class => "class",
            Self::Order => "order",
            Self::Family => "family",
            Self::Tribe => "tribe",
            Self::Genus => "genus",
            Self::Species => "species",
            Self::Varietas => "varietas",
        }
    }

    /// Integer score used when turning a deepest shared rank into a taxonomic bonus.
    pub fn score(self) -> u8 {
        match self {
            Self::Domain => 1,
            Self::Kingdom => 2,
            Self::Phylum => 3,
            Self::Class => 4,
            Self::Order => 5,
            Self::Family => 6,
            Self::Tribe => 7,
            Self::Genus => 8,
            Self::Species => 9,
            Self::Varietas => 10,
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Domain => 0,
            Self::Kingdom => 1,
            Self::Phylum => 2,
            Self::Class => 3,
            Self::Order => 4,
            Self::Family => 5,
            Self::Tribe => 6,
            Self::Genus => 7,
            Self::Species => 8,
            Self::Varietas => 9,
        }
    }
}

/// Taxonomic lineage with one optional value per supported rank.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaxonomyLineage {
    ranks: [Option<String>; 10],
}

impl TaxonomyLineage {
    /// Creates a lineage directly from rank-ordered optional values.
    pub fn from_rank_values(values: [Option<String>; 10]) -> Self {
        Self { ranks: values }
    }

    /// Returns the value stored for a given rank, if present.
    pub fn value_for(&self, rank: TaxonomicRank) -> Option<&str> {
        self.ranks[rank.index()].as_deref()
    }

    /// Returns the specificity score of the deepest populated rank.
    pub fn specificity_score(&self) -> usize {
        TaxonomicRank::ALL
            .iter()
            .rev()
            .find_map(|rank| self.value_for(*rank).map(|_| rank.score() as usize))
            .unwrap_or(0)
    }

    /// Returns the deepest rank shared by both lineages, if any.
    pub fn deepest_shared_rank(&self, other: &Self) -> Option<TaxonomicRank> {
        TaxonomicRank::ALL.iter().rev().copied().find(|rank| {
            match (self.value_for(*rank), other.value_for(*rank)) {
                (Some(left), Some(right)) => left == right,
                _ => false,
            }
        })
    }

    /// Returns a copy of the lineage truncated at the requested rank.
    pub fn truncated_to(&self, rank: TaxonomicRank) -> Self {
        let mut ranks: [Option<String>; 10] = Default::default();
        for candidate in TaxonomicRank::ALL {
            if candidate.index() > rank.index() {
                break;
            }
            ranks[candidate.index()] = self.value_for(candidate).map(ToOwned::to_owned);
        }
        Self { ranks }
    }

    fn merge_prefer_more_specific(&mut self, other: &Self) {
        if other.specificity_score() > self.specificity_score() {
            *self = other.clone();
        }
    }
}

/// Organism metadata attached to a short InChIKey entry in LOTUS.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LotusBiosource {
    pub organism_name: String,
    pub organism_wikidata: Option<String>,
    pub lineage: TaxonomyLineage,
}

/// Query lineage resolved from a user-supplied organism or taxon name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedLotusQuery {
    pub query_label: String,
    pub lineage: TaxonomyLineage,
}

/// Accepted OpenTree taxon returned by strict TNRS validation.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, PartialEq)]
pub struct OtolAcceptedTaxon {
    pub input_name: String,
    pub accepted_name: String,
    pub matched_name: String,
    pub ott_id: Option<u64>,
    pub rank: Option<String>,
    pub score: f64,
}

/// Candidate OpenTree match used in validation errors and tests.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, PartialEq)]
pub struct OtolTaxonMatch {
    pub accepted_name: String,
    pub matched_name: String,
    pub ott_id: Option<u64>,
    pub rank: Option<String>,
    pub score: f64,
    pub is_synonym: bool,
    pub is_approximate_match: bool,
    pub is_suppressed: bool,
}

/// Best taxonomic match found for a library candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaxonomicMatch {
    pub score: u8,
    pub shared_rank: Option<TaxonomicRank>,
    pub matched_organism_name: Option<String>,
    pub matched_organism_wikidata: Option<String>,
    pub matched_short_inchikey: Option<String>,
}

/// Index built from LOTUS metadata for resolving query taxa and matching candidates.
#[derive(Clone, Debug, Default)]
pub struct LotusMetadataIndex {
    by_short_inchikey: HashMap<String, Vec<LotusBiosource>>,
    by_organism_name: HashMap<String, TaxonomyLineage>,
    by_organism_wikidata: HashMap<String, TaxonomyLineage>,
    by_taxon_name: HashMap<String, TaxonomyLineage>,
}

impl LotusMetadataIndex {
    /// Resolves a user-provided query string into a lineage using LOTUS metadata.
    pub fn resolve_query_lineage(&self, input: &str) -> Option<ResolvedLotusQuery> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return None;
        }

        if let Some(qid) = normalized_qid(trimmed) {
            let lineage = self.by_organism_wikidata.get(&qid)?.clone();
            return Some(ResolvedLotusQuery {
                query_label: qid,
                lineage,
            });
        }

        if let Some(lineage) = self.by_organism_name.get(trimmed) {
            return Some(ResolvedLotusQuery {
                query_label: trimmed.to_string(),
                lineage: lineage.clone(),
            });
        }

        let lineage = self.by_taxon_name.get(trimmed)?.clone();
        Some(ResolvedLotusQuery {
            query_label: trimmed.to_string(),
            lineage,
        })
    }

    /// Returns the best lineage match for a short InChIKey against the query lineage.
    pub fn match_candidate(
        &self,
        short_inchikey: &str,
        query_lineage: &TaxonomyLineage,
    ) -> Option<TaxonomicMatch> {
        let biosources = self.by_short_inchikey.get(short_inchikey)?;
        let mut best: Option<TaxonomicMatch> = None;

        for biosource in biosources {
            let shared_rank = biosource.lineage.deepest_shared_rank(query_lineage);
            let score = shared_rank.map(TaxonomicRank::score).unwrap_or(0);
            let candidate = TaxonomicMatch {
                score,
                shared_rank,
                matched_organism_name: Some(biosource.organism_name.clone()),
                matched_organism_wikidata: biosource.organism_wikidata.clone(),
                matched_short_inchikey: Some(short_inchikey.to_string()),
            };

            let replace = match &best {
                None => true,
                Some(current) => {
                    candidate.score > current.score
                        || (candidate.score == current.score
                            && biosource.lineage.specificity_score()
                                > current
                                    .shared_rank
                                    .map(|rank| rank.score() as usize)
                                    .unwrap_or(0))
                        || (candidate.score == current.score
                            && candidate.matched_organism_name.as_deref()
                                < current.matched_organism_name.as_deref())
                }
            };
            if replace {
                best = Some(candidate);
            }
        }

        best
    }
}

/// Parses LOTUS metadata from in-memory CSV bytes.
pub fn load_lotus_bytes(bytes: &[u8]) -> Result<LotusMetadataIndex, String> {
    parse_lotus_reader(bytes)
}

/// Loads LOTUS metadata from a CSV file on disk.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_lotus_path(path: &std::path::Path) -> Result<LotusMetadataIndex, String> {
    let bytes =
        std::fs::read(path).map_err(|err| format!("cannot read {}: {err}", path.display()))?;
    load_lotus_bytes(&bytes)
}

/// Validates a taxon name against the OpenTree Taxonomic Name Resolution Service.
#[cfg(not(target_arch = "wasm32"))]
pub fn validate_otol_taxon_name(
    query: &str,
    context_name: Option<&str>,
) -> Result<OtolAcceptedTaxon, String> {
    let query = query.trim();
    if query.is_empty() {
        return Err("OTOL validation requires a non-empty taxon query".to_string());
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .user_agent(concat!(
            "spectral-matcher/",
            env!("CARGO_PKG_VERSION"),
            " taxon-validation"
        ))
        .build()
        .map_err(|err| format!("failed to build OTOL HTTP client: {err}"))?;
    let response = client
        .post(OTOL_TNRS_MATCH_NAMES_URL)
        .json(&OtolMatchNamesRequest {
            names: vec![query],
            context_name,
            do_approximate_matching: true,
            include_suppressed: false,
        })
        .send()
        .map_err(|err| format!("failed to validate '{query}' with OTOL TNRS: {err}"))?
        .error_for_status()
        .map_err(|err| format!("OTOL TNRS rejected validation for '{query}': {err}"))?
        .json::<OtolMatchNamesResponse>()
        .map_err(|err| format!("failed to parse OTOL TNRS response for '{query}': {err}"))?;

    validate_otol_response(query, response)
}

/// Extracts a short InChIKey candidate from a spectrum record's headers.
pub fn short_inchikey_from_record<T>(record: &SpectrumRecord<T>) -> Option<String> {
    let mut best: Option<(usize, String)> = None;
    for (key, value) in &record.meta.headers {
        let normalized = normalize_key(key);
        let priority = if normalized == "ik2d" {
            0
        } else if normalized == "structureinchikey" || normalized == "gnpsinchikey" {
            1
        } else if normalized.contains("inchikey") {
            2
        } else {
            continue;
        };
        let Some(short) = short_inchikey(value) else {
            continue;
        };
        match &best {
            Some((best_priority, _)) if *best_priority <= priority => {}
            _ => best = Some((priority, short)),
        }
    }
    best.map(|(_, value)| value)
}

pub fn short_inchikey(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_matches('"');
    if trimmed.is_empty() {
        return None;
    }
    let upper = trimmed.to_ascii_uppercase();
    let compact: String = upper.chars().filter(|ch| *ch != '-').collect();
    if compact.len() < 14 {
        return None;
    }
    Some(compact.chars().take(14).collect())
}

fn parse_lotus_reader<R: Read>(reader: R) -> Result<LotusMetadataIndex, String> {
    let mut csv = csv::ReaderBuilder::new().flexible(true).from_reader(reader);
    let headers = csv
        .headers()
        .map_err(|err| format!("failed to read LOTUS header: {err}"))?
        .clone();

    let short_inchikey_idx = header_index(&headers, "structure_inchikey")?;
    let organism_name_idx = header_index(&headers, "organism_name")?;
    let organism_wikidata_idx = header_index(&headers, "organism_wikidata")?;
    let taxonomy_indices = TAXONOMY_COLUMN_NAMES
        .iter()
        .map(|name| header_index(&headers, name))
        .collect::<Result<Vec<_>, _>>()?;

    let mut by_short_inchikey: HashMap<String, Vec<LotusBiosource>> = HashMap::new();
    let mut by_organism_name: HashMap<String, TaxonomyLineage> = HashMap::new();
    let mut by_organism_wikidata: HashMap<String, TaxonomyLineage> = HashMap::new();
    let mut by_taxon_name: HashMap<String, TaxonomyLineage> = HashMap::new();

    for record in csv.records() {
        let record = record.map_err(|err| format!("failed to parse LOTUS row: {err}"))?;

        let Some(short_key) = record.get(short_inchikey_idx).and_then(short_inchikey) else {
            continue;
        };
        let organism_name = record
            .get(organism_name_idx)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("")
            .to_string();
        if organism_name.is_empty() {
            continue;
        }

        let organism_wikidata = record
            .get(organism_wikidata_idx)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);

        let mut rank_values: [Option<String>; 10] = Default::default();
        for (slot, idx) in rank_values.iter_mut().zip(taxonomy_indices.iter().copied()) {
            *slot = record
                .get(idx)
                .map(str::trim)
                .filter(|value| !value.is_empty() && *value != "NA")
                .map(ToOwned::to_owned);
        }
        let lineage = TaxonomyLineage::from_rank_values(rank_values);
        let biosource = LotusBiosource {
            organism_name: organism_name.clone(),
            organism_wikidata: organism_wikidata.clone(),
            lineage: lineage.clone(),
        };
        by_short_inchikey
            .entry(short_key)
            .or_default()
            .push(biosource);

        by_organism_name
            .entry(organism_name)
            .and_modify(|existing| existing.merge_prefer_more_specific(&lineage))
            .or_insert(lineage.clone());

        if let Some(qid) = organism_wikidata.and_then(|value| normalized_qid(&value)) {
            by_organism_wikidata
                .entry(qid)
                .and_modify(|existing| existing.merge_prefer_more_specific(&lineage))
                .or_insert(lineage.clone());
        }

        for rank in TaxonomicRank::ALL {
            let Some(name) = lineage.value_for(rank) else {
                continue;
            };
            let truncated = lineage.truncated_to(rank);
            by_taxon_name
                .entry(name.to_string())
                .and_modify(|existing| existing.merge_prefer_more_specific(&truncated))
                .or_insert(truncated);
        }
    }

    Ok(LotusMetadataIndex {
        by_short_inchikey,
        by_organism_name,
        by_organism_wikidata,
        by_taxon_name,
    })
}

fn header_index(headers: &csv::StringRecord, target: &str) -> Result<usize, String> {
    headers
        .iter()
        .position(|header| header.trim() == target)
        .ok_or_else(|| format!("LOTUS file is missing required column '{target}'"))
}

fn normalize_key(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_lowercase())
        .collect()
}

fn normalized_qid(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let candidate = trimmed
        .rsplit('/')
        .next()
        .filter(|segment| !segment.is_empty())
        .unwrap_or(trimmed);
    let upper = candidate.to_ascii_uppercase();
    let suffix = upper.strip_prefix('Q')?;
    if suffix.chars().all(|ch| ch.is_ascii_digit()) {
        Some(format!("Q{suffix}"))
    } else {
        None
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(serde::Serialize)]
struct OtolMatchNamesRequest<'a> {
    names: Vec<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context_name: Option<&'a str>,
    do_approximate_matching: bool,
    include_suppressed: bool,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(serde::Deserialize)]
struct OtolMatchNamesResponse {
    #[serde(default)]
    unmatched_names: Vec<String>,
    #[serde(default)]
    results: Vec<OtolNameResult>,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(serde::Deserialize)]
struct OtolNameResult {
    #[serde(default)]
    matches: Vec<OtolApiMatch>,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(serde::Deserialize)]
struct OtolApiMatch {
    #[serde(default)]
    matched_name: Option<String>,
    #[serde(default)]
    score: Option<f64>,
    #[serde(default)]
    is_approximate_match: bool,
    #[serde(default)]
    is_synonym: bool,
    #[serde(default)]
    taxon: Option<OtolApiTaxon>,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(serde::Deserialize)]
struct OtolApiTaxon {
    #[serde(default)]
    ott_id: Option<u64>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    unique_name: Option<String>,
    #[serde(default)]
    rank: Option<String>,
    #[serde(default)]
    is_suppressed: bool,
}

#[cfg(not(target_arch = "wasm32"))]
fn validate_otol_response(
    query: &str,
    response: OtolMatchNamesResponse,
) -> Result<OtolAcceptedTaxon, String> {
    let candidates = otol_candidates(response.results);
    let accepted: Vec<&OtolTaxonMatch> = candidates
        .iter()
        .filter(|candidate| is_strict_otol_acceptance(candidate))
        .collect();

    match accepted.as_slice() {
        [candidate] => Ok(OtolAcceptedTaxon {
            input_name: query.to_string(),
            accepted_name: candidate.accepted_name.clone(),
            matched_name: candidate.matched_name.clone(),
            ott_id: candidate.ott_id,
            rank: candidate.rank.clone(),
            score: candidate.score,
        }),
        [] => Err(format_otol_validation_error(
            query,
            &response.unmatched_names,
            &candidates,
        )),
        _ => Err(format!(
            "OTOL validation for '{}' is ambiguous; choose one accepted match explicitly: {}",
            query,
            format_otol_candidates(&candidates)
        )),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn is_strict_otol_acceptance(candidate: &OtolTaxonMatch) -> bool {
    candidate.score >= 0.999_999
        && !candidate.is_approximate_match
        && !candidate.is_synonym
        && !candidate.is_suppressed
}

#[cfg(not(target_arch = "wasm32"))]
fn otol_candidates(results: Vec<OtolNameResult>) -> Vec<OtolTaxonMatch> {
    let mut candidates = results
        .into_iter()
        .flat_map(|result| {
            result.matches.into_iter().filter_map(move |matched| {
                let taxon = matched.taxon?;
                let accepted_name = taxon.name.or(taxon.unique_name)?;
                let matched_name = matched
                    .matched_name
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| accepted_name.clone());
                Some(OtolTaxonMatch {
                    accepted_name,
                    matched_name,
                    ott_id: taxon.ott_id,
                    rank: taxon.rank,
                    score: matched.score.unwrap_or(0.0),
                    is_synonym: matched.is_synonym,
                    is_approximate_match: matched.is_approximate_match,
                    is_suppressed: taxon.is_suppressed,
                })
            })
        })
        .collect::<Vec<_>>();

    candidates.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.is_approximate_match.cmp(&right.is_approximate_match))
            .then_with(|| left.is_synonym.cmp(&right.is_synonym))
            .then_with(|| left.is_suppressed.cmp(&right.is_suppressed))
            .then_with(|| left.accepted_name.cmp(&right.accepted_name))
    });
    candidates
}

#[cfg(not(target_arch = "wasm32"))]
fn format_otol_validation_error(
    query: &str,
    unmatched_names: &[String],
    candidates: &[OtolTaxonMatch],
) -> String {
    if candidates.is_empty() {
        if unmatched_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(query))
        {
            return format!("OTOL validation failed for '{query}': no match found");
        }
        return format!("OTOL validation failed for '{query}': no acceptable match found");
    }

    format!(
        "OTOL validation failed for '{}': query is not an exact accepted OTOL name; suggested matches: {}",
        query,
        format_otol_candidates(candidates)
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn format_otol_candidates(candidates: &[OtolTaxonMatch]) -> String {
    candidates
        .iter()
        .take(5)
        .map(|candidate| {
            let ott_id = candidate
                .ott_id
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            let rank = candidate.rank.as_deref().unwrap_or("unknown");
            format!(
                "{} (matched '{}', ott_id {}, rank {}, score {:.3}, synonym {}, approximate {}, suppressed {})",
                candidate.accepted_name,
                candidate.matched_name,
                ott_id,
                rank,
                candidate.score,
                candidate.is_synonym,
                candidate.is_approximate_match,
                candidate.is_suppressed
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::{TaxonomicRank, load_lotus_bytes};

    fn sample_lotus() -> super::LotusMetadataIndex {
        let csv = concat!(
            "structure_inchikey,organism_wikidata,organism_name,organism_taxonomy_01domain,organism_taxonomy_02kingdom,organism_taxonomy_03phylum,organism_taxonomy_04class,organism_taxonomy_05order,organism_taxonomy_06family,organism_taxonomy_07tribe,organism_taxonomy_08genus,organism_taxonomy_09species,organism_taxonomy_10varietas\n",
            "\"ABCDEFGHIJKLMN-AAAA\",http://www.wikidata.org/entity/Q1,\"Withania somnifera\",Eukaryota,Archaeplastida,Streptophyta,Magnoliopsida,Solanales,Solanaceae,NA,Withania,Withania somnifera,NA\n",
            "\"ABCDEFGHIJKLMN-BBBB\",http://www.wikidata.org/entity/Q2,\"Withania coagulans\",Eukaryota,Archaeplastida,Streptophyta,Magnoliopsida,Solanales,Solanaceae,NA,Withania,Withania coagulans,NA\n",
            "\"ZZZZZZZZZZZZZZ-CCCC\",http://www.wikidata.org/entity/Q3,\"Panax ginseng\",Eukaryota,Archaeplastida,Streptophyta,Magnoliopsida,Apiales,Araliaceae,NA,Panax,Panax ginseng,NA\n",
        );
        load_lotus_bytes(csv.as_bytes()).expect("lotus")
    }

    #[test]
    fn resolves_query_lineage_by_name_qid_and_genus() {
        let lotus = sample_lotus();

        let by_name = lotus
            .resolve_query_lineage("Withania somnifera")
            .expect("name lineage");
        assert_eq!(
            by_name.lineage.value_for(TaxonomicRank::Species),
            Some("Withania somnifera")
        );

        let by_qid = lotus.resolve_query_lineage("Q1").expect("qid lineage");
        assert_eq!(by_qid.query_label, "Q1");
        assert_eq!(
            by_qid.lineage.value_for(TaxonomicRank::Genus),
            Some("Withania")
        );

        let by_genus = lotus
            .resolve_query_lineage("Withania")
            .expect("genus lineage");
        assert_eq!(by_genus.query_label, "Withania");
        assert_eq!(
            by_genus.lineage.value_for(TaxonomicRank::Family),
            Some("Solanaceae")
        );
        assert_eq!(
            by_genus.lineage.value_for(TaxonomicRank::Genus),
            Some("Withania")
        );
        assert_eq!(by_genus.lineage.value_for(TaxonomicRank::Species), None);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn otol_validation_accepts_exact_non_synonym() {
        let response = serde_json::from_str::<super::OtolMatchNamesResponse>(
            r#"{
              "unmatched_names": [],
              "results": [{
                "name": "Withania somnifera",
                "matches": [{
                  "matched_name": "Withania somnifera",
                  "score": 1.0,
                  "is_approximate_match": false,
                  "is_synonym": false,
                  "taxon": {
                    "ott_id": 512345,
                    "name": "Withania somnifera",
                    "rank": "species",
                    "is_suppressed": false
                  }
                }]
              }]
            }"#,
        )
        .expect("response");

        let accepted =
            super::validate_otol_response("Withania somnifera", response).expect("accepted");
        assert_eq!(accepted.accepted_name, "Withania somnifera");
        assert_eq!(accepted.ott_id, Some(512345));
        assert_eq!(accepted.rank.as_deref(), Some("species"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn otol_validation_rejects_synonym_with_corrected_match() {
        let response = serde_json::from_str::<super::OtolMatchNamesResponse>(
            r#"{
              "unmatched_names": [],
              "results": [{
                "name": "Old name",
                "matches": [{
                  "matched_name": "Old name",
                  "score": 1.0,
                  "is_approximate_match": false,
                  "is_synonym": true,
                  "taxon": {
                    "ott_id": 42,
                    "name": "Accepted name",
                    "rank": "species",
                    "is_suppressed": false
                  }
                }]
              }]
            }"#,
        )
        .expect("response");

        let err = super::validate_otol_response("Old name", response).expect_err("rejected");
        assert!(err.contains("Accepted name"));
        assert!(err.contains("synonym true"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn otol_validation_rejects_approximate_match() {
        let response = serde_json::from_str::<super::OtolMatchNamesResponse>(
            r#"{
              "unmatched_names": [],
              "results": [{
                "name": "Withania somnifer",
                "matches": [{
                  "matched_name": "Withania somnifera",
                  "score": 0.98,
                  "is_approximate_match": true,
                  "is_synonym": false,
                  "taxon": {
                    "ott_id": 512345,
                    "name": "Withania somnifera",
                    "rank": "species",
                    "is_suppressed": false
                  }
                }]
              }]
            }"#,
        )
        .expect("response");

        let err =
            super::validate_otol_response("Withania somnifer", response).expect_err("rejected");
        assert!(err.contains("Withania somnifera"));
        assert!(err.contains("approximate true"));
    }
}
