//! Joins positive MZmine feature abundances to biological sample lineages.

use std::borrow::Cow;
use std::collections::HashMap;

use crate::api::FeatureTaxonomySources;
use crate::taxonomy::{LotusMetadataIndex, TaxonomyLineage};
#[derive(Debug)]
pub(crate) struct FeatureTaxonomyIndex {
    pub(crate) samples: Vec<TaxonomyLineage>,
    pub(crate) features: HashMap<String, Vec<usize>>,
}

fn input_bytes<'a>(
    text: Option<&'a str>,
    path: Option<&str>,
    name: &str,
) -> Result<Cow<'a, [u8]>, String> {
    if let Some(text) = text {
        return Ok(Cow::Borrowed(text.as_bytes()));
    }
    if let Some(path) = path {
        #[cfg(not(target_arch = "wasm32"))]
        {
            return std::fs::read(path)
                .map(Cow::Owned)
                .map_err(|err| format!("cannot read {name} {path}: {err}"));
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = path;
            return Err(format!("{name} path loading is unavailable on wasm"));
        }
    }
    Err(format!("missing {name} CSV text or path"))
}

fn required_column(headers: &csv::StringRecord, name: &str) -> Result<usize, String> {
    headers
        .iter()
        .position(|header| header == name)
        .ok_or_else(|| format!("missing required CSV column '{name}'"))
}

fn metadata_lineage(
    row: &csv::StringRecord,
    rank_columns: &[Option<usize>; 10],
    taxon_column: usize,
    lotus: &LotusMetadataIndex,
) -> Option<TaxonomyLineage> {
    let values = rank_columns.map(|column| {
        column
            .and_then(|index| row.get(index))
            .map(str::trim)
            .filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case("NA"))
            .map(str::to_owned)
    });
    if values.iter().any(Option::is_some) {
        return Some(TaxonomyLineage::from_rank_values(values));
    }
    row.get(taxon_column)
        .map(str::trim)
        .filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case("NA"))
        .and_then(|name| lotus.resolve_query_lineage(name))
        .map(|query| query.lineage)
}

pub(crate) fn load_feature_taxonomy(
    sources: &FeatureTaxonomySources,
    lotus: &LotusMetadataIndex,
) -> Result<FeatureTaxonomyIndex, String> {
    let metadata_bytes = input_bytes(
        sources.metadata_csv_text.as_deref(),
        sources.metadata_csv_path.as_deref(),
        "sample metadata",
    )?;
    let mut metadata = csv::Reader::from_reader(metadata_bytes.as_ref());
    let headers = metadata
        .headers()
        .map_err(|err| format!("sample metadata headers: {err}"))?;
    let filename_column = required_column(headers, "filename")?;
    let taxon_column = required_column(headers, &sources.taxon_column)?;
    let sample_type_column = headers.iter().position(|header| header == "sample_type");
    let rank_names = [
        "col_domain",
        "col_kingdom",
        "col_phylum",
        "col_class",
        "col_order",
        "ATTRIBUTE_col_family",
        "col_tribe",
        "ATTRIBUTE_col_genus",
        "col_species",
        "col_varietas",
    ];
    let rank_columns = rank_names.map(|name| headers.iter().position(|header| header == name));
    let mut samples = Vec::new();
    let mut by_filename = HashMap::new();
    for row in metadata.records() {
        let row = row.map_err(|err| format!("sample metadata row: {err}"))?;
        let filename = row.get(filename_column).unwrap_or("").trim();
        if filename.is_empty() {
            continue;
        }
        if let Some(column) = sample_type_column {
            if !row
                .get(column)
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case("sample")
            {
                continue;
            }
        }
        let Some(lineage) = metadata_lineage(&row, &rank_columns, taxon_column, lotus) else {
            continue;
        };
        let index = samples.len();
        if by_filename.insert(filename.to_string(), index).is_some() {
            return Err(format!("duplicate sample metadata filename '{filename}'"));
        }
        samples.push(lineage);
    }

    let quant_bytes = input_bytes(
        sources.quant_csv_text.as_deref(),
        sources.quant_csv_path.as_deref(),
        "feature quantification",
    )?;
    let mut quant = csv::Reader::from_reader(quant_bytes.as_ref());
    let headers = quant
        .headers()
        .map_err(|err| format!("feature quantification headers: {err}"))?;
    let feature_column = required_column(headers, "row ID")?;
    let sample_columns: Vec<_> = headers
        .iter()
        .enumerate()
        .filter_map(|(column, header)| {
            header
                .strip_suffix(" Peak area")
                .and_then(|filename| by_filename.get(filename).map(|&sample| (column, sample)))
        })
        .collect();
    if sample_columns.is_empty() {
        return Err(
            "no feature quantification sample columns match biological metadata filenames"
                .to_string(),
        );
    }
    let mut features = HashMap::new();
    for row in quant.records() {
        let row = row.map_err(|err| format!("feature quantification row: {err}"))?;
        let id = row.get(feature_column).unwrap_or("").trim();
        if id.is_empty() {
            return Err("feature quantification row has an empty row ID".to_string());
        }
        let mut contributors = Vec::new();
        for &(column, sample) in &sample_columns {
            let value = row.get(column).unwrap_or("").trim();
            if !value.is_empty() {
                let intensity: f64 = value.parse().map_err(|err| {
                    format!("feature '{id}' has invalid intensity '{value}': {err}")
                })?;
                if intensity.is_finite() && intensity > 0.0 {
                    contributors.push(sample);
                }
            }
        }
        if features.insert(id.to_string(), contributors).is_some() {
            return Err(format!("duplicate feature quantification row ID '{id}'"));
        }
    }
    Ok(FeatureTaxonomyIndex { samples, features })
}
