# CLAUDE.md — vgi-sourcemap

Contributor/agent notes. User-facing docs live in `README.md`; this is the
"how it's built and where the sharp edges are" companion.

## What this is

A [VGI](https://query.farm) worker (Rust, compiled binary) that decodes
**JavaScript Source Map v3** (`.map`) files and resolves minified `(line, col)`
positions back to their original `source:line:col:name`, exposed to DuckDB/SQL
over Arrow IPC (`ATTACH 'sourcemap' (TYPE vgi, LOCATION '…')`). Functions live
under catalog `sourcemap`, schema `main`. Built on the published VGI Rust SDK
(`vgi = "0.46"` from crates.io), arrow 60; modeled on `../vgi-units` and
`../vgi-fixedformat`. Builds standalone — no local SDK checkout.

**Deliberately thin** (committee scope ≈ 1 week): the Sentry `sourcemap` crate
(BSD-3-Clause) does all the real work — Base64-VLQ decode, index maps
(`sections`), inline `sourcesContent`, the nearest-preceding `lookup_token`. We
never reimplement any of it; the worker is row-mapping + the `lookup_all`
table-in-out plumbing.

## Layout

```
Cargo.toml                                 workspace; pins vgi 0.46, arrow 60, sourcemap 9
crates/sourcemap-worker/
  src/main.rs                              Worker::new(); registers scalars/tables/table-in-out + catalog metadata
  src/lib.rs                               lib target re-exporting `smap` for integration tests
  src/smap.rs                              PURE engine (no Arrow): decode_slice → flatten → TokenRow/Resolved/SourceRow/Meta + unit tests
  src/maparg.rs                            overloaded `map` arg (path | JSON text | BLOB) → bytes
  src/cache.rs                             last-value decode cache (parse one map once across a column of rows)
  src/arrow_io.rs                          int/pos cell reads, the resolved STRUCT fields, commented-Field helper, test harness
  src/meta.rs                              vgi-lint metadata-tag helpers (keywords_json / object_tags / agent_test_tasks_json)
  src/scalar/{lookup,source_content,version,mod}.rs   thin Arrow scalar adapters
  src/table/{tokens,sources,map_meta,mod}.rs          thin Arrow table-producer adapters
  src/table_in_out/{lookup_all,mod}.rs                the bulk frame-resolution exchange function
  tests/golden.rs                          fixture-driven engine tests (data/*.map)
  tests/fuzz.rs                            proptest: the decoder NEVER panics on arbitrary/truncated input
data/*.map                                 golden fixtures: basic, index (sections), inline sourcesContent, webpack-style
test/sql/*.test                            haybarn sqllogictest E2E (authoritative)
ci/                                        run-integration.sh + preprocess-require.awk + check-version.sh
```

Pattern: keep decoding in `smap.rs` (pure, unit-tested), keep Arrow marshalling
in `arrow_io.rs` + `maparg.rs` + the `scalar/`/`table/`/`table_in_out/` adapters
(thin, harness-tested). The bin (`main.rs`) and lib (`lib.rs`) both compile
`smap.rs` — `main.rs` has `mod smap;`, `lib.rs` re-exports `pub mod smap;` for
`tests/` (the vgi-units idiom).

## The decode model

`Smap::parse(bytes)` calls `sourcemap::decode_slice` (which strips the `)]}'`
XSSI guard and handles regular / index / Hermes maps), then **normalizes to a
single flattened `SourceMap`**: index maps flatten via `SourceMapIndex::flatten`,
Hermes maps reduce to their embedded regular map. Token/lookup/source access is
then uniform. `meta()` reports `version` (always 3), `file`, `source_root`, and
counts (`n_sources`/`n_names`/`n_mappings` over the flattened token list).

`get_source` already joins the map's `sourceRoot` onto each path (so a
`sourceRoot` of `/src` yields `/src/index.js`) — that's the resolved path you
want for de-minification, and `meta.source_root` still surfaces the root
separately.

## Sharp edges

1. **All coordinates are 0-indexed** (Source Map v3 / the crate). Both the
   generated `(line, col)` you pass to `lookup`/`lookup_all` and every `*_line` /
   `*_col` output. Document this loudly; browser stack frames are usually
   1-indexed.

2. **Nearest-preceding, not exact.** `lookup_token` returns the greatest token
   `<= (line, col)`. A genuine **no-match → NULL** therefore only happens when
   the position *precedes the first* mapping; a position *after* all mappings
   resolves to the *last* token (that's why the basic fixture's first token
   starts at generated column 2, so `(0,0)` is a real no-match in tests).

3. **NULL-vs-error policy.** Per-row resolution (`lookup`, `lookup_all`) treats a
   NULL/malformed/empty/unreadable map, a NULL position, or a no-match as **NULL
   resolution** — never a query error — so a column of dirty frames doesn't abort
   a scan. The whole-map table functions (`tokens`/`sources`/`meta`) take a
   **constant** map and DO error on a bad path/parse (the call is about that one
   map). The split lives in `maparg.rs` (per-cell, tolerant) vs
   `table::load_const_map` (strict).

4. **The overloaded `map` arg.** `maparg::map_bytes_cell` decides path-vs-text by
   a leading `{` (or `)]}'`); everything else is read as a file path. `BLOB`
   columns are used verbatim. There is **no HTTP fetch** of `sourceMappingURL` —
   the caller supplies the map.

5. **`lookup_all` is table-in-out.** Input relation `(SELECT id, line, col FROM
   …)` + named `map :=` / `id :=` / `line :=` / `col :=` (defaults `id`/`line`/
   `col`). `on_bind` clones the input `id` field verbatim (passthrough keeps its
   name AND type) and validates the line/col columns exist; `process` decodes the
   single map once per batch and emits `[id, source, src_line, src_col, name]`. A
   bad map → all rows NULL resolution but the id still passes through.

6. **`read_text` is a TABLE function in DuckDB**, not a scalar — so pass the map
   **path directly** (`lookup('app.min.js.map', …)`), which the worker reads, or
   inline JSON text. The E2E `files.test` uses path mode for exactly this reason.

7. **Decode cache.** `cache::MapCache` memoizes the last-decoded map keyed by a
   hash of its bytes, so a column of rows referencing the same deployed map
   parses it once. Hash-only key (negligible collision risk, accepted).

## Testing

```sh
cargo test --workspace --all-features    # smap unit + golden fixtures + proptest fuzz + arrow-boundary
cargo clippy --all-targets --all-features -- -D warnings && cargo fmt --all -- --check
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace
# E2E (subprocess|http|unix), needs haybarn-unittest + community vgi extension:
HAYBARN_UNITTEST="$(command -v haybarn-unittest)" \
  WORKER_BIN="$PWD/target/release/sourcemap-worker" TRANSPORT=subprocess ci/run-integration.sh
```

`test/sql/*.test` use `LOAD vgi;` (not `require vgi`) + `require-env
VGI_SOURCEMAP_WORKER`. Scalar/table tests `SET search_path='sourcemap.main'`;
`lookup_all.test` does NOT (it `CREATE`s a temp table, so it fully-qualifies the
`sourcemap.main.lookup_all` call instead of pointing search_path at the
read-only worker catalog). Fixtures in `data/` are staged next to the runner so
path-mode tests resolve.

## Metadata / vgi-lint

Per-object discovery tags come from `meta::object_tags`; the catalog/schema
metadata + `source_url` + agent-test tasks live in `main.rs`. Every example query,
executable example, and agent task runs against **inline map text**, so they bind
AND execute offline — the worker passes clean (0 findings at info, score 100) at `vgi-lint --fail-on info`
(see `vgi-lint.toml`). Watch out for VGI313 (don't name a data type in an
argument description) and VGI310 (don't type every argument `ANY` — the position
args are `BIGINT`, only the genuinely-overloaded `map` is `ANY`).

Rule notes worth keeping in mind:
- **VGI515** — a described example (`{description, sql}`) is required per object.
  A `FunctionExample`'s description is dropped by `duckdb_functions().examples`,
  so each function ALSO emits a byte-identical `vgi.example_queries` tag
  (`meta::example_queries_json`); the schema's `vgi.example_queries` is likewise a
  described-JSON list, not a newline-joined SQL blob.
- **VGI328** — no parameterless `*_version()` scalar. The build version is the
  catalog's `implementation_version` (set in `catalog_metadata`, read from
  `vgi_catalogs()`); `version()` in `main.rs` feeds it.
- **VGI327** — no self-cataloguing `functions` registry view (it duplicates
  `duckdb_functions()`).
- **VGI146** — a worker with table functions still needs ONE browsable table/view,
  so `example_tokens` (a data view decoding the built-in `SAMPLE_MAP` into token
  rows) is that entry point. It is NOT a function registry (its columns are token
  data, not name/kind/category), so it satisfies VGI146 without tripping VGI327.
- **VGI182** — backtick DuckDB type names in prose docs (`BLOB`, `STRUCT(...)`).
- **VGI131** — numeric column comments want a unit/definition hint (a `(`
  parenthetical satisfies it); it checks table/view columns, not table-function
  result columns.

## Function surface

Scalars: `lookup` (→ STRUCT(source, src_line, src_col, name)), `source_content`
(→ VARCHAR). Table functions: `tokens`, `sources`, `meta`. Table-in-out:
`lookup_all` (bulk, id passthrough). One browsable view: `example_tokens` (the
decoded token rows of the built-in `SAMPLE_MAP`, for discovery). The running build
is published as the catalog's `implementation_version`, not a scalar.
