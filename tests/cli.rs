#![cfg(not(target_arch = "wasm32"))]
//! Integration tests for the command-line interface.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use spectral_matcher::{
    ComputeParams, ConsensusArtifact, LibrarySearchParams, ParseStats, SearchArtifact,
    SearchArtifactHit, SearchArtifactResult, SearchQueryKey, SimilarityMetric, SpectrumMetadata,
};

/// Creates a unique temporary directory for a test case.
fn temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock drift")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("spectral_matcher_{label}_{nanos}"));
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// Writes a UTF-8 text fixture to disk.
fn write_file(path: &PathBuf, contents: &str) {
    fs::write(path, contents).expect("write file");
}

/// Produces a tiny one-spectrum MGF payload for CLI fixture generation.
fn sample_mgf(name: &str) -> String {
    format!("BEGIN IONS\nNAME={name}\nPEPMASS=100.0\n10 100\n20 80\n30 50\nEND IONS\n")
}

/// Writes a minimal search artifact fixture for consensus CLI tests.
fn write_search_artifact(
    path: &PathBuf,
    library_source_label: &str,
    library_spectra: Vec<SpectrumMetadata>,
    hits: Vec<SearchArtifactHit>,
) {
    let queries = vec![SpectrumMetadata {
        id: 0,
        label: "query_0".to_string(),
        raw_name: "query_0".to_string(),
        feature_id: Some("feature_0".to_string()),
        scans: Some("scan_0".to_string()),
        filename: None,
        source_scan_usi: None,
        featurelist_feature_id: None,
        headers: BTreeMap::new(),
        precursor_mz: 100.0,
        num_peaks: 3,
    }];
    let artifact = SearchArtifact {
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
        taxonomy: None,
        query_key: SearchQueryKey::FeatureId,
        query_spectra: queries,
        library_spectra,
        result: SearchArtifactResult {
            hits,
            query_count: 1,
            library_count: 2,
            metric: SimilarityMetric::LinearCosine,
            taxonomic_reranking_applied: true,
            taxonomic_query: Some("Withania somnifera".to_string()),
        },
        tsv: String::new(),
    };
    write_file(
        path,
        &serde_json::to_string_pretty(&artifact).expect("serialize search artifact"),
    );
}

/// Builds minimal library metadata used in search-artifact fixtures.
fn library_meta(id: usize, raw_name: &str, inchikey: &str) -> SpectrumMetadata {
    let mut headers = BTreeMap::new();
    headers.insert("INCHIKEY".to_string(), inchikey.to_string());
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

/// Ensures the curated database registry is exposed through the CLI.
#[test]
fn db_list_cli_lists_seeded_databases() {
    let output = Command::new(env!("CARGO_BIN_EXE_spectral-matcher"))
        .arg("db")
        .arg("list")
        .output()
        .expect("run db list cli");
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert!(stdout.contains("Available spectral databases:"));
    assert!(stdout.contains("all_gnps_no_propogated"));
    assert!(stdout.contains("all_gnps_no_propogated_matchms"));
    assert!(stdout.contains("isdb_lotus_pos_energysum"));
}

/// Ensures the CLI advertises the supported spectral similarity metrics.
#[test]
fn metrics_cli_lists_available_metrics() {
    let output = Command::new(env!("CARGO_BIN_EXE_spectral-matcher"))
        .arg("metrics")
        .output()
        .expect("run metrics cli");
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert!(stdout.contains("Available similarity metrics:"));
    assert!(stdout.contains("HungarianCosine (default)"));
    assert!(stdout.contains("ModifiedHungarianCosine"));
    assert!(stdout.contains("LinearEntropyWeighted"));
}

/// Verifies that the search command can write both JSON and TSV outputs.
#[test]
fn search_cli_writes_json_and_optional_tsv() {
    let dir = temp_dir("search_json_tsv");
    let query = dir.join("query.mgf");
    let library = dir.join("library.mgf");
    let config = dir.join("config.toml");
    let output_root = dir.join("out");
    let output_json = output_root.join("test/search.json");
    let output_tsv = output_root.join("test/search.tsv");
    write_file(&query, &sample_mgf("query"));
    write_file(&library, &sample_mgf("library"));
    write_file(
        &config,
        &format!(
            r#"
output_dir = "{}"

[[jobs]]
name = "test"
query_mgf = "{}"
library_mgf = "{}"

[jobs.parse]
min_peaks = 1
max_peaks = 1000

[jobs.search]
metric = "LinearCosine"
precursor_mz_tolerance = 1.0
fragment_mz_tolerance = 0.2
mz_power = 0.0
intensity_power = 1.0
min_matched_peaks = 1
min_similarity_threshold = 0.0
top_n = 1
"#,
            output_root.display(),
            query.display(),
            library.display(),
        ),
    );

    let output = Command::new(env!("CARGO_BIN_EXE_spectral-matcher"))
        .arg("search")
        .arg("--config")
        .arg(&config)
        .output()
        .expect("run cli");
    assert!(output.status.success(), "{output:?}");

    let json = fs::read_to_string(output_json).expect("json output");
    let tsv = fs::read_to_string(output_tsv).expect("tsv output");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
    assert_eq!(parsed["result"]["hits"].as_array().map(Vec::len), Some(1));
    assert!(tsv.starts_with("query_export_key\tquery_key_mode"));
    let _ = fs::remove_dir_all(dir);
}

/// Verifies that batch configs execute every declared search job.
#[test]
fn search_cli_runs_multiple_jobs() {
    let dir = temp_dir("search_batch");
    let query = dir.join("query.mgf");
    let library = dir.join("library.mgf");
    let config = dir.join("config.toml");
    let output_one = dir.join("out/one.json");
    let output_two = dir.join("out/two.json");
    write_file(&query, &sample_mgf("query"));
    write_file(&library, &sample_mgf("library"));
    write_file(
        &config,
        &format!(
            r#"
[[jobs]]
name = "one"
query_mgf = "{}"
library_mgf = "{}"
output_json = "{}"

[jobs.parse]
min_peaks = 1
max_peaks = 1000

[jobs.search]
metric = "LinearCosine"
precursor_mz_tolerance = 1.0
fragment_mz_tolerance = 0.2
mz_power = 0.0
intensity_power = 1.0
min_matched_peaks = 1
min_similarity_threshold = 0.0
top_n = 1

[[jobs]]
name = "two"
query_mgf = "{}"
library_mgf = "{}"
output_json = "{}"

[jobs.parse]
min_peaks = 1
max_peaks = 1000

[jobs.search]
metric = "LinearCosine"
precursor_mz_tolerance = 1.0
fragment_mz_tolerance = 0.2
mz_power = 0.0
intensity_power = 1.0
min_matched_peaks = 1
min_similarity_threshold = 0.0
top_n = 1
"#,
            query.display(),
            library.display(),
            output_one.display(),
            query.display(),
            library.display(),
            output_two.display(),
        ),
    );

    let output = Command::new(env!("CARGO_BIN_EXE_spectral-matcher"))
        .arg("search")
        .arg("--config")
        .arg(&config)
        .output()
        .expect("run cli");
    assert!(output.status.success(), "{output:?}");
    assert!(output_one.exists());
    assert!(output_two.exists());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn search_cli_applies_taxonomic_reranking_when_taxonomy_config_is_present() {
    let dir = temp_dir("search_taxonomy");
    let query = dir.join("query.mgf");
    let library = dir.join("library.mgf");
    let lotus = dir.join("lotus.csv");
    let config = dir.join("config.toml");
    let output_root = dir.join("out");
    let output_json = output_root.join("taxonomy/search.json");
    let output_tsv = output_root.join("taxonomy/search.tsv");
    write_file(
        &query,
        "BEGIN IONS\nNAME=q\nFEATURE_ID=1\nPEPMASS=100.0\n10 100\n20 80\n30 50\nEND IONS\n",
    );
    write_file(
        &library,
        concat!(
            "BEGIN IONS\nNAME=withania\nPEPMASS=100.0\n",
            "INCHIKEY=AAAAAAAAAAAAAA-111\n",
            "10 100\n21 80\n35 30\nEND IONS\n",
            "BEGIN IONS\nNAME=panax\nPEPMASS=100.0\n",
            "INCHIKEY=BBBBBBBBBBBBBB-222\n",
            "10 100\n20 80\n30 50\nEND IONS\n",
        ),
    );
    write_file(
        &lotus,
        concat!(
            "structure_inchikey,organism_wikidata,organism_name,organism_taxonomy_01domain,organism_taxonomy_02kingdom,organism_taxonomy_03phylum,organism_taxonomy_04class,organism_taxonomy_05order,organism_taxonomy_06family,organism_taxonomy_07tribe,organism_taxonomy_08genus,organism_taxonomy_09species,organism_taxonomy_10varietas\n",
            "\"AAAAAAAAAAAAAA-111\",http://www.wikidata.org/entity/Q1,\"Withania somnifera\",Eukaryota,Archaeplastida,Streptophyta,Magnoliopsida,Solanales,Solanaceae,NA,Withania,Withania somnifera,NA\n",
            "\"BBBBBBBBBBBBBB-222\",http://www.wikidata.org/entity/Q2,\"Panax ginseng\",Eukaryota,Archaeplastida,Streptophyta,Magnoliopsida,Apiales,Araliaceae,NA,Panax,Panax ginseng,NA\n",
        ),
    );
    write_file(
        &config,
        &format!(
            r#"
output_dir = "{}"

[[jobs]]
name = "taxonomy"
query_mgf = "{}"
library_mgf = "{}"

[jobs.parse]
min_peaks = 1
max_peaks = 1000

[jobs.search]
metric = "LinearCosine"
precursor_mz_tolerance = 0.1
fragment_mz_tolerance = 0.2
mz_power = 0.0
intensity_power = 1.0
min_matched_peaks = 1
min_similarity_threshold = 0.0
top_n = 1

[jobs.taxonomy]
query = "Withania somnifera"
lotus_csv = "{}"
"#,
            output_root.display(),
            query.display(),
            library.display(),
            lotus.display(),
        ),
    );

    let output = Command::new(env!("CARGO_BIN_EXE_spectral-matcher"))
        .arg("search")
        .arg("--config")
        .arg(&config)
        .output()
        .expect("run cli");
    assert!(output.status.success(), "{output:?}");

    let json = fs::read_to_string(output_json).expect("json output");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
    assert_eq!(parsed["result"]["taxonomic_reranking_applied"], true);
    assert_eq!(parsed["result"]["taxonomic_query"], "Withania somnifera");
    assert_eq!(parsed["result"]["hits"][0]["library_index"], 0);
    assert_eq!(parsed["result"]["hits"][0]["taxonomic_score"], 9.0);

    let tsv = fs::read_to_string(output_tsv).expect("tsv output");
    let header = tsv.lines().next().expect("tsv header");
    assert!(header.contains("hit_taxonomic_score"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn consensus_cli_merges_two_search_artifacts_into_one_annotation_per_query() {
    let dir = temp_dir("consensus");
    let left = dir.join("gnps.json");
    let right = dir.join("isdb.json");
    let config = dir.join("config.toml");
    let output_root = dir.join("out");
    let output_json = output_root.join("merged/consensus.json");
    let output_tsv = output_root.join("merged/consensus.tsv");
    write_search_artifact(
        &left,
        "gnps.mgf",
        vec![
            library_meta(0, "singleton", "BBBBBBBBBBBBBB-AAAA"),
            library_meta(1, "consensus_gnps", "AAAAAAAAAAAAAA-BBBB"),
        ],
        vec![
            SearchArtifactHit {
                query_index: 0,
                library_index: 0,
                rank: 1,
                rank_before_taxonomy: Some(2),
                spectral_score: 0.9,
                ms1_deviation_ppm: 1.0,
                taxonomic_score: 1.0,
                combined_score: 1.9,
                matches: 6,
                matched_organism_name: Some("Withania somnifera".to_string()),
                matched_organism_wikidata: Some("Q1".to_string()),
                matched_shared_rank: Some("species".to_string()),
                matched_short_inchikey: Some("BBBBBBBBBBBBBB".to_string()),
            },
            SearchArtifactHit {
                query_index: 0,
                library_index: 1,
                rank: 3,
                rank_before_taxonomy: Some(4),
                spectral_score: 0.7,
                ms1_deviation_ppm: 2.0,
                taxonomic_score: 1.0,
                combined_score: 1.7,
                matches: 6,
                matched_organism_name: Some("Withania somnifera".to_string()),
                matched_organism_wikidata: Some("Q1".to_string()),
                matched_shared_rank: Some("species".to_string()),
                matched_short_inchikey: Some("AAAAAAAAAAAAAA".to_string()),
            },
        ],
    );
    write_search_artifact(
        &right,
        "isdb.mgf",
        vec![
            library_meta(0, "other_singleton", "CCCCCCCCCCCCCC-DDDD"),
            library_meta(1, "consensus_isdb", "AAAAAAAAAAAAAA-CCCC"),
        ],
        vec![
            SearchArtifactHit {
                query_index: 0,
                library_index: 0,
                rank: 1,
                rank_before_taxonomy: Some(2),
                spectral_score: 0.85,
                ms1_deviation_ppm: 1.0,
                taxonomic_score: 1.0,
                combined_score: 1.85,
                matches: 6,
                matched_organism_name: Some("Withania somnifera".to_string()),
                matched_organism_wikidata: Some("Q1".to_string()),
                matched_shared_rank: Some("species".to_string()),
                matched_short_inchikey: Some("CCCCCCCCCCCCCC".to_string()),
            },
            SearchArtifactHit {
                query_index: 0,
                library_index: 1,
                rank: 2,
                rank_before_taxonomy: Some(3),
                spectral_score: 0.75,
                ms1_deviation_ppm: 2.0,
                taxonomic_score: 1.0,
                combined_score: 1.75,
                matches: 6,
                matched_organism_name: Some("Withania somnifera".to_string()),
                matched_organism_wikidata: Some("Q1".to_string()),
                matched_shared_rank: Some("species".to_string()),
                matched_short_inchikey: Some("AAAAAAAAAAAAAA".to_string()),
            },
        ],
    );
    write_file(
        &config,
        &format!(
            r#"
output_dir = "{}"

[[jobs]]
name = "merged"
left_search_json = "{}"
right_search_json = "{}"
left_name = "gnps"
right_name = "isdb"
"#,
            output_root.display(),
            left.display(),
            right.display(),
        ),
    );

    let output = Command::new(env!("CARGO_BIN_EXE_spectral-matcher"))
        .arg("consensus")
        .arg("--config")
        .arg(&config)
        .output()
        .expect("run consensus cli");
    assert!(output.status.success(), "{output:?}");

    let json = fs::read_to_string(output_json).expect("json output");
    let parsed: ConsensusArtifact = serde_json::from_str(&json).expect("valid consensus json");
    let annotation = parsed.result.queries[0]
        .annotation
        .as_ref()
        .expect("annotation");
    assert_eq!(annotation.consensus_key.as_deref(), Some("AAAAAAAAAAAAAA"));
    assert_eq!(annotation.support_count, 2);
    assert_eq!(annotation.best_rank_by_input["gnps"], 3);
    assert_eq!(annotation.best_rank_by_input["isdb"], 2);

    let tsv = fs::read_to_string(output_tsv).expect("tsv output");
    let header = tsv.lines().next().expect("tsv header");
    assert!(header.contains("best_rank_gnps"));
    assert!(header.contains("best_rank_isdb"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn network_cli_writes_network_json_and_csvs() {
    let dir = temp_dir("network");
    let input = dir.join("query.mgf");
    let config = dir.join("config.toml");
    let output_root = dir.join("out");
    let output_json = output_root.join("network/network.json");
    let output_csv_dir = output_root.join("network/csv");
    write_file(
        &input,
        concat!(
            "BEGIN IONS\nNAME=a\nFEATURE_ID=1\nPEPMASS=100.0\n10 100\n20 80\n30 50\nEND IONS\n",
            "BEGIN IONS\nNAME=b\nFEATURE_ID=2\nPEPMASS=100.1\n10 100\n20 80\n30 50\nEND IONS\n"
        ),
    );
    write_file(
        &config,
        &format!(
            r#"
output_dir = "{}"

[[jobs]]
name = "network"
input_mgf = "{}"

[jobs.parse]
min_peaks = 1
max_peaks = 1000

[jobs.build.compute]
metric = "LinearCosine"
fragment_mz_tolerance = 0.2
mz_power = 0.0
intensity_power = 1.0

[jobs.build]
threshold = 0.0
top_k = 5
"#,
            output_root.display(),
            input.display(),
        ),
    );

    let output = Command::new(env!("CARGO_BIN_EXE_spectral-matcher"))
        .arg("network")
        .arg("--config")
        .arg(&config)
        .output()
        .expect("run network cli");
    assert!(output.status.success(), "{output:?}");
    assert!(output_json.exists());
    assert!(output_csv_dir.join("nodes.csv").exists());
    assert!(output_csv_dir.join("edges.csv").exists());
    let nodes_csv = fs::read_to_string(output_csv_dir.join("nodes.csv")).expect("nodes csv");
    let edges_csv = fs::read_to_string(output_csv_dir.join("edges.csv")).expect("edges csv");
    assert!(nodes_csv.starts_with("node_id,precursor_mz,num_peaks,component_id,degree\n"));
    assert!(nodes_csv.contains("1,100.000000,3,0,1\n"));
    assert!(nodes_csv.contains("2,100.100000,3,0,1\n"));
    assert!(edges_csv.contains("1,2,"));
    let _ = fs::remove_dir_all(dir);
}
