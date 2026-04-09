//! Curated spectral-database registry and native download helpers.

/// Describes a downloadable spectral database exposed by the CLI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpectralDatabase {
    /// Stable CLI identifier.
    pub id: &'static str,
    /// User-facing display name.
    pub name: &'static str,
    /// Default output filename.
    pub filename: &'static str,
    /// Source URL used for downloading.
    pub url: &'static str,
    /// High-level database category such as experimental or in silico.
    pub category: &'static str,
    /// Short dimensionality or coverage label.
    pub dimensions: &'static str,
    /// Human-readable description shown by `db list`.
    pub description: &'static str,
}

const SPECTRAL_DATABASES: [SpectralDatabase; 3] = [
    SpectralDatabase {
        id: "all_gnps_no_propogated",
        name: "ALL_GNPS_NO_PROPOGATED",
        filename: "gnps_cleaned.mgf",
        url: "https://external.gnps2.org/processed_gnps_data/gnps_cleaned.mgf",
        category: "Experimental",
        dimensions: "GNPS-wide, cleaned MGF",
        description: "Experimental GNPS library without propagated annotations.",
    },
    SpectralDatabase {
        id: "all_gnps_no_propogated_matchms",
        name: "ALL_GNPS_NO_PROPOGATED_MATCHMS",
        filename: "matchms.mgf",
        url: "https://external.gnps2.org/processed_gnps_data/matchms.mgf",
        category: "Experimental",
        dimensions: "GNPS-wide, MatchMS-cleaned MGF",
        description: "Experimental GNPS library cleaned with GNPS processing plus MatchMS.",
    },
    SpectralDatabase {
        id: "isdb_lotus_pos_energysum",
        name: "ISDB_LOTUS_POS_ENERGYSUM",
        filename: "isdb_lotus_pos_energySum.mgf",
        url: "https://zenodo.org/records/14887271/files/isdb_lotus_pos_energySum.mgf?download=1",
        category: "In silico",
        dimensions: "positive mode, CFM-ID energySum",
        description: "CFM-ID predicted positive-mode ISDB/LOTUS library.",
    },
];

/// Returns the built-in curated spectral database registry.
pub fn spectral_databases() -> &'static [SpectralDatabase] {
    &SPECTRAL_DATABASES
}

/// Resolves a database by id, display name, or filename.
pub fn resolve_spectral_database(input: &str) -> Option<&'static SpectralDatabase> {
    let needle = normalize_label(input);
    spectral_databases().iter().find(|database| {
        normalize_label(database.id) == needle
            || normalize_label(database.name) == needle
            || normalize_label(database.filename) == needle
    })
}

fn normalize_label(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut prev_sep = true;
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_sep = false;
        } else if !prev_sep {
            out.push('_');
            prev_sep = true;
        }
    }
    out.trim_matches('_').to_string()
}

/// Downloads a curated spectral database into `output_dir`, reporting byte progress as data arrives.
#[cfg(not(target_arch = "wasm32"))]
pub fn download_spectral_database<F>(
    database: &SpectralDatabase,
    output_dir: &std::path::Path,
    mut on_progress: F,
) -> Result<std::path::PathBuf, String>
where
    F: FnMut(u64, Option<u64>),
{
    use std::io::{Read, Write};

    std::fs::create_dir_all(output_dir)
        .map_err(|err| format!("failed to create {}: {err}", output_dir.display()))?;
    let destination = output_dir.join(database.filename);
    let partial = output_dir.join(format!("{}.part", database.filename));

    let client = reqwest::blocking::Client::builder()
        .build()
        .map_err(|err| format!("failed to build HTTP client: {err}"))?;
    let mut response = client
        .get(database.url)
        .send()
        .map_err(|err| format!("failed to start download from {}: {err}", database.url))?
        .error_for_status()
        .map_err(|err| format!("download failed for {}: {err}", database.url))?;

    let total = response.content_length();
    on_progress(0, total);

    let mut file = std::fs::File::create(&partial)
        .map_err(|err| format!("failed to create {}: {err}", partial.display()))?;
    let mut downloaded = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = response
            .read(&mut buffer)
            .map_err(|err| format!("failed while downloading {}: {err}", database.url))?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|err| format!("failed to write {}: {err}", partial.display()))?;
        downloaded = downloaded.saturating_add(read as u64);
        on_progress(downloaded, total);
    }

    file.flush()
        .map_err(|err| format!("failed to flush {}: {err}", partial.display()))?;
    std::fs::rename(&partial, &destination).map_err(|err| {
        format!(
            "failed to move {} to {}: {err}",
            partial.display(),
            destination.display()
        )
    })?;
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::resolve_spectral_database;

    #[test]
    fn resolve_database_accepts_id_name_and_filename() {
        assert_eq!(
            resolve_spectral_database("all_gnps_no_propogated")
                .map(|database| database.filename),
            Some("gnps_cleaned.mgf")
        );
        assert_eq!(
            resolve_spectral_database("ALL_GNPS_NO_PROPOGATED_MATCHMS")
                .map(|database| database.filename),
            Some("matchms.mgf")
        );
        assert_eq!(
            resolve_spectral_database("isdb_lotus_pos_energySum.mgf")
                .map(|database| database.id),
            Some("isdb_lotus_pos_energysum")
        );
    }
}
