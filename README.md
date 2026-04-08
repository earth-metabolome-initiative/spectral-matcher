# spectral-matcher

Rust crate and CLI for:

- building spectral networks from MGF files
- running spectral-library matching against external MGF libraries
- serving those operations over a local HTTP API for `spectral-network-gui`

## Build

From the repository root:

```bash
cargo build
cargo test
```

## CLI Overview

The CLI supports three commands:

```bash
cargo run -- serve
cargo run -- network --config path/to/network.toml
cargo run -- search --config path/to/search.toml
```

Important: the `network` and `search` commands are config-file driven. They do not accept MGF paths directly on the command line.

## Run The Local Matcher Service

This is only needed if you want `spectral-network-gui` to submit jobs to the matcher service directly.

```bash
cargo run -- serve
```

By default the service binds to `127.0.0.1:8787`.

## Build A Spectral Network

Use a config file like this:

```toml
[[jobs]]
name = "mapp_batch_00231"
input_mgf = "fixtures/mapp_batch_00231.mgf"
output_json = "out/mapp_batch_00231/network.json"
output_csv_dir = "out/mapp_batch_00231/csv"

[jobs.parse]
min_peaks = 5
max_peaks = 1000

[jobs.build.compute]
metric = "CosineGreedy"
tolerance = 0.2
mz_power = 0.0
intensity_power = 1.0

[jobs.build]
threshold = 0.7
top_k = 10
```

Then run:

```bash
cargo run -- network --config config/network.toml
```

Outputs:

- `out/mapp_batch_00231/network.json`
- `out/mapp_batch_00231/csv/nodes.csv`
- `out/mapp_batch_00231/csv/edges.csv`

Notes:

- `threshold` is the minimum similarity score required to keep an edge.
- `top_k` is the maximum number of retained neighbors per node.
- The CLI can appear quiet while computing; for larger jobs that is expected.

## Run Spectral Matching Against An External Library

For your current setup, use:

- query MGF: `fixtures/mapp_batch_00231.mgf`
- library MGF: `/Users/pma/01_large_files/mgf/isdb_lotus_pos_energySum.mgf`

Example config:

```toml
[[jobs]]
name = "mapp_vs_isdb_lotus"
query_mgf = "fixtures/mapp_batch_00231.mgf"
library_mgf = "/Users/pma/01_large_files/mgf/isdb_lotus_pos_energySum.mgf"
output_json = "out/mapp_vs_isdb_lotus/search.json"
output_tsv = "out/mapp_vs_isdb_lotus/search.tsv"

[jobs.parse]
min_peaks = 5
max_peaks = 1000

[jobs.search]
metric = "CosineGreedy"
tolerance = 0.2
mz_power = 0.0
intensity_power = 1.0
precursor_mz_tolerance = 0.05
min_matched_peaks = 3
min_similarity_threshold = 0.7
top_n = 20

[jobs.output]
query_key = "FeatureId"
```

Then run:

```bash
cargo run -- search --config config/search.toml
```

Outputs:

- `out/mapp_vs_isdb_lotus/search.json`
- `out/mapp_vs_isdb_lotus/search.tsv`

The TSV uses a compact query identity schema:

- `query_export_key`: the query identifier chosen by `query_key`
- `query_key_mode`: what that identifier represents, for example `FEATURE_ID`
- the remaining columns describe the ranked library hit

Parameter guidance:

- Start with `tolerance = 0.2` unless you have a reason to tighten or relax fragment matching.
- `precursor_mz_tolerance = 0.05` is a reasonable first pass for precursor filtering.
- `min_matched_peaks = 3` avoids many weak accidental hits.
- `min_similarity_threshold = 0.7` is fairly strict. If you get too few hits, try `0.6` or `0.5`.
- `top_n = 20` keeps the best 20 matches per query spectrum.

## Load Results In spectral-network-gui

The GUI loads matcher JSON artifacts directly. It does not use the exported CSV files as input.

### Load A Network Build

1. Start the GUI.
2. Click `Load matcher network JSON`.
3. Select:

```text
out/mapp_batch_00231/network.json
```

### Load A Library Search Result

1. Start the GUI.
2. Click `Load matcher search JSON`.
3. Select:

```text
out/mapp_vs_isdb_lotus/search.json
```

Alternatively, you can start the matcher service with `cargo run -- serve` and let the GUI submit network/search jobs directly.

## Exported Network CSVs

When exporting a network:

- `nodes.csv` uses a single `node_id` column
- `node_id` is taken from `FEATURE_ID` when present
- otherwise it falls back to the internal node index plus one
- `edges.csv` uses the same exported identifiers in `source` and `target`

## Current Local Examples

This repository currently contains:

- query fixture: `fixtures/mapp_batch_00231.mgf`
- example network config: `config/network.toml`
- example search config: `config/search.toml`

The current search config already targets:

- query: `fixtures/mapp_batch_00231.mgf`
- library: `/Users/pma/01_large_files/mgf/isdb_lotus_pos_energySum.mgf`
