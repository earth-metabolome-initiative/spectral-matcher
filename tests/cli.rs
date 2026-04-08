#![cfg(not(target_arch = "wasm32"))]

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock drift")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("spectral_matcher_{label}_{nanos}"));
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn write_file(path: &PathBuf, contents: &str) {
    fs::write(path, contents).expect("write file");
}

fn sample_mgf(name: &str) -> String {
    format!("BEGIN IONS\nNAME={name}\nPEPMASS=100.0\n10 100\n20 80\n30 50\nEND IONS\n")
}

#[test]
fn metrics_cli_lists_available_metrics() {
    let output = Command::new(env!("CARGO_BIN_EXE_spectral-matcher"))
        .arg("metrics")
        .output()
        .expect("run metrics cli");
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert!(stdout.contains("Available similarity metrics:"));
    assert!(stdout.contains("CosineGreedy (default)"));
    assert!(stdout.contains("ModifiedCosine"));
    assert!(stdout.contains("LinearEntropyWeighted"));
}

#[test]
fn search_cli_writes_json_and_optional_tsv() {
    let dir = temp_dir("search_json_tsv");
    let query = dir.join("query.mgf");
    let library = dir.join("library.mgf");
    let config = dir.join("config.toml");
    let output_json = dir.join("out/result.json");
    let output_tsv = dir.join("out/result.tsv");
    write_file(&query, &sample_mgf("query"));
    write_file(&library, &sample_mgf("library"));
    write_file(
        &config,
        &format!(
            r#"
[[jobs]]
name = "test"
query_mgf = "{}"
library_mgf = "{}"
output_json = "{}"
output_tsv = "{}"

[jobs.parse]
min_peaks = 1
max_peaks = 1000

[jobs.search]
metric = "CosineGreedy"
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
            output_json.display(),
            output_tsv.display(),
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
metric = "CosineGreedy"
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
metric = "CosineGreedy"
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
    let output_json = dir.join("out/result.json");
    let output_tsv = dir.join("out/result.tsv");
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
[[jobs]]
name = "taxonomy"
query_mgf = "{}"
library_mgf = "{}"
output_json = "{}"
output_tsv = "{}"

[jobs.parse]
min_peaks = 1
max_peaks = 1000

[jobs.search]
metric = "CosineGreedy"
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
            query.display(),
            library.display(),
            output_json.display(),
            output_tsv.display(),
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
fn network_cli_writes_network_json_and_csvs() {
    let dir = temp_dir("network");
    let input = dir.join("query.mgf");
    let config = dir.join("config.toml");
    let output_json = dir.join("out/network.json");
    let output_csv_dir = dir.join("out/csv");
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
[[jobs]]
name = "network"
input_mgf = "{}"
output_json = "{}"
output_csv_dir = "{}"

[jobs.parse]
min_peaks = 1
max_peaks = 1000

[jobs.build.compute]
metric = "CosineGreedy"
fragment_mz_tolerance = 0.2
mz_power = 0.0
intensity_power = 1.0

[jobs.build]
threshold = 0.0
top_k = 5
"#,
            input.display(),
            output_json.display(),
            output_csv_dir.display(),
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
