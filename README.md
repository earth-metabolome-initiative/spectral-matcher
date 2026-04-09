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

The CLI supports these commands:

```bash
cargo run -- serve
cargo run -- network --config path/to/network.toml
cargo run -- search --config path/to/search.toml
cargo run -- consensus --config path/to/consensus.toml
cargo run -- metrics
cargo run -- db list
cargo run -- db download <database-id>
```

Important: the `network` and `search` commands are config-file driven. They do not accept MGF paths directly on the command line.

To see the currently supported similarity metrics and their meanings:

```bash
cargo run -- metrics
```

This prints the valid `metric = "..."` values for both network and search configs.

To list and download curated spectral databases:

```bash
cargo run -- db list
cargo run -- db download all_gnps_no_propogated
cargo run -- db download isdb_lotus_pos_energysum --output-dir /path/to/databases
```

The downloader shows a colored progress bar for interactive terminals and writes the selected MGF into `databases/` by default.

## Run The Local Matcher Service

This is only needed if you want `spectral-network-gui` to submit jobs to the matcher service directly.

```bash
cargo run -- serve
```

By default the service binds to `127.0.0.1:8787`.

## Build A Spectral Network

Use a config file like this:

```toml
output_dir = "out"

[[jobs]]
name = "mapp_batch_00231"
input_mgf = "fixtures/mapp_batch_00231.mgf"

[jobs.parse]
min_peaks = 5
max_peaks = 1000

[jobs.build.compute]
metric = "CosineGreedy"
fragment_mz_tolerance = 0.2
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

With `output_dir = "out"`, the CLI derives output locations from each job `name`. For network jobs it writes `<output_dir>/<name>/network.json` and `<output_dir>/<name>/csv/`.

Notes:

- `threshold` is the minimum similarity score required to keep an edge.
- `top_k` is the maximum number of retained neighbors per node.
- The CLI can appear quiet while computing; for larger jobs that is expected.

## Available Metrics

- `CosineHungarian`: exact cosine matching with Hungarian assignment; slower, more exhaustive.
- `CosineGreedy`: fast cosine matching with greedy assignment; default choice for most runs.
- `ModifiedCosine`: precursor-shift-aware cosine using Hungarian assignment for analog-style matching.
- `ModifiedGreedyCosine`: faster precursor-shift-aware cosine using greedy assignment.
- `LinearEntropyWeighted`: spectral entropy similarity with intensity weighting after entropy preprocessing.
- `LinearEntropyUnweighted`: spectral entropy similarity without intensity weighting after entropy preprocessing.
- `ModifiedLinearEntropyWeighted`: precursor-shift-aware spectral entropy similarity with intensity weighting.
- `ModifiedLinearEntropyUnweighted`: precursor-shift-aware spectral entropy similarity without intensity weighting.

## Run Spectral Matching Against An External Library

For your current setup, use:

- query MGF: `fixtures/mapp_batch_00231.mgf`
- library MGF: `/Users/pma/01_large_files/mgf/isdb_lotus_pos_energySum.mgf`

Example config:

```toml
output_dir = "out"

[[jobs]]
name = "mapp_vs_isdb_lotus"
query_mgf = "fixtures/mapp_batch_00231.mgf"
library_mgf = "/Users/pma/01_large_files/mgf/isdb_lotus_pos_energySum.mgf"

[jobs.parse]
min_peaks = 5
max_peaks = 1000

[jobs.search]
metric = "CosineGreedy"
precursor_mz_tolerance = 0.05
fragment_mz_tolerance = 0.2
mz_power = 0.0
intensity_power = 1.0
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

With `output_dir = "out"`, the CLI derives search outputs as `<output_dir>/<name>/search.json` and `<output_dir>/<name>/search.tsv`.

If you need custom locations, you can still set explicit `output_json`, `output_tsv`, or `output_csv_dir` fields on individual jobs.

The TSV uses a compact query identity schema:

- `query_export_key`: the query identifier chosen by `query_key`
- `query_key_mode`: what that identifier represents, for example `FEATURE_ID`
- the remaining columns describe the ranked library hit

Parameter guidance:

- Start with `fragment_mz_tolerance = 0.2` unless you have a reason to tighten or relax fragment matching.
- `precursor_mz_tolerance = 0.05` is a reasonable first pass for precursor filtering.
- `min_matched_peaks = 3` avoids many weak accidental hits.
- `min_similarity_threshold = 0.7` is fairly strict. If you get too few hits, try `0.6` or `0.5`.
- `top_n = 20` keeps the best 20 matches per query spectrum.

## Merge Two Search Outputs Into One Consensus Annotation Per Query

Use this when you want one merged annotation per spectrum for GUI display, while still boosting molecules supported by multiple libraries.

Example config:

```toml
output_dir = "out"

[[jobs]]
name = "mapp_gnps_isdb_consensus"
left_search_json = "out/mapp_vs_gnps_lotus/search.json"
right_search_json = "out/mapp_vs_isdb_lotus/search.json"
left_name = "gnps"
right_name = "isdb"

[jobs.merge]
top_k_per_library = 5
rrf_k = 10.0
consensus_bonus = 0.05
left_weight = 1.0
right_weight = 1.0

[jobs.output]
query_key = "FeatureId"
```

Then run:

```bash
cargo run -- consensus --config config/consensus.toml
```

Outputs:

- `out/mapp_gnps_isdb_consensus/consensus.json`
- `out/mapp_gnps_isdb_consensus/consensus.tsv`

Behavior:

- groups candidate annotations by short InChIKey
- keeps only the top `K` hits from each input artifact before merging
- applies reciprocal-rank fusion plus a cross-library consensus bonus
- emits one row per query spectrum
- preserves singleton winners when only one library supports the annotation

The merged JSON keeps provenance for each winning annotation, including the supporting libraries, best rank per input, representative structure metadata, and whether the agreement is exact-structure consensus or only short-InChIKey/scaffold consensus.

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
- example consensus config: `config/consensus.toml`

The current search config already targets:

- query: `fixtures/mapp_batch_00231.mgf`
- library: `/Users/pma/01_large_files/mgf/isdb_lotus_pos_energySum.mgf`
