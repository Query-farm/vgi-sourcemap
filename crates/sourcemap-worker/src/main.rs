//! The `sourcemap` VGI worker.
//!
//! A standalone binary DuckDB launches and talks to over Apache Arrow IPC
//! (`ATTACH 'sourcemap' (TYPE vgi, LOCATION '…')`). It decodes **JavaScript
//! Source Map v3** (`.map`) files — the Base64-VLQ `mappings` grammar — into rows
//! and resolves minified `(line, col)` positions back to their original
//! `source:line:col:name`, under the catalog `sourcemap`, schema `main`:
//!
//! ```sql
//! ATTACH 'sourcemap' (TYPE vgi, LOCATION './target/release/sourcemap-worker');
//! SET search_path = 'sourcemap.main';
//!
//! SELECT * FROM tokens('/dist/app.min.js.map');                 -- decode mappings
//! SELECT lookup(read_text('app.min.js.map'), 0, 12);            -- resolve one frame
//! SELECT * FROM sources('/dist/app.min.js.map');                -- original files
//! SELECT * FROM meta('/dist/app.min.js.map');                   -- map metadata
//! ```
//!
//! The pure decode/resolution engine (a thin wrapper over the Sentry `sourcemap`
//! crate) lives in [`smap`]; the `scalar/`, `table/`, and `table_in_out/` modules
//! are thin Arrow adapters over it.

mod arrow_io;
mod cache;
mod maparg;
mod meta;
mod scalar;
mod smap;
mod table;
mod table_in_out;

use vgi::catalog::{CatSchema, CatalogModel};
use vgi::Worker;

/// Worker version string, surfaced by `sourcemap_version()`.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// A small inline v3 map reused by the catalog's example SQL so every snippet
/// runs without a `.map` file present (avoids the file-dependent zero-rows lint
/// findings). One source, one mapping.
const SAMPLE_MAP: &str =
    "{\"version\":3,\"file\":\"o.js\",\"sources\":[\"index.js\"],\"names\":[],\"mappings\":\"AAAA\"}";

/// Catalog + schema metadata (description, provenance) surfaced to DuckDB and the
/// `vgi-lint` metadata-quality linter. The function objects themselves are served
/// from the registered scalars / table / table-in-out functions; this only adds
/// catalog/schema-level comments and tags.
fn catalog_metadata(name: &str) -> CatalogModel {
    CatalogModel {
        name: name.to_string(),
        comment: Some(
            "Decode JavaScript Source Map v3 (.map) files and resolve minified positions back to \
             original source locations."
                .to_string(),
        ),
        tags: vec![
            (
                "vgi.title".to_string(),
                "Source Map Decoder & Position Resolver".to_string(),
            ),
            (
                "vgi.keywords".to_string(),
                crate::meta::keywords_json(
                    "source map, sourcemap, source-map, .map, v3, Base64 VLQ, VLQ, mappings, \
                     de-minify, deminify, minified, unminify, resolve, lookup, symbolicate, \
                     stack trace, stack frame, RUM, error tracking, webpack, esbuild, rollup, \
                     terser, sourcesContent, index map, sections, original source",
                ),
            ),
            (
                "vgi.doc_llm".to_string(),
                "Decode JavaScript Source Map v3 (.map) files in SQL and resolve minified \
                 (line, col) positions back to their original source:line:col:name. `tokens(map)` \
                 explodes a map into its decoded mapping table; `lookup(map, gen_line, gen_col)` \
                 resolves one minified position to a STRUCT(source, src_line, src_col, name) using \
                 nearest-preceding semantics; `lookup_all((SELECT id, line, col FROM frames), \
                 map := …, id := 'id')` bulk-resolves a whole column of error frames with the id \
                 passed through to join back; `sources(map)` lists original files (with an inline \
                 sourcesContent flag) and `source_content(map, src_id)` returns embedded source \
                 text; `meta(map)` reports version/file/source_root and counts. The `map` argument \
                 is a path to a .map file, the raw map JSON text, or map BLOB bytes. Index maps \
                 (sections) and inline sourcesContent are supported. All line/column numbers are \
                 0-indexed. Use it to de-minify production RUM / error-tracking stack frames in \
                 bulk with one JOIN."
                    .to_string(),
            ),
            (
                "vgi.doc_md".to_string(),
                "# sourcemap\n\nDecode **JavaScript Source Map v3** (`.map`) files and resolve \
                 minified `(line, col)` positions back to original `source:line:col:name`, \
                 directly in DuckDB SQL. Nothing else in DuckDB reads source maps — `read_json` \
                 gives you the wrapper object but not the decoded Base64-VLQ `mappings`. This \
                 worker turns a column of `(error_url, line, col)` RUM frames into resolved \
                 original locations with one JOIN.\n\nThe heavy lifting (Base64-VLQ decode, index \
                 maps via `sections`, inline `sourcesContent`, the nearest-preceding lookup) is \
                 done by the Sentry `sourcemap` crate; this worker is a thin Arrow adapter over \
                 it. The overloaded `map` argument accepts a **path** to a `.map` file, the raw \
                 map **JSON text** (e.g. from `read_text(...)`), or map **BLOB** bytes — so \
                 data-URI inline maps decode via the text/bytes mode. All line/column numbers are \
                 **0-indexed** (Source Map v3).\n\n**Scalars:** `lookup(map, gen_line, gen_col)` \
                 (→ STRUCT(source, src_line, src_col, name)), `source_content(map, src_id)` \
                 (→ VARCHAR), `sourcemap_version()`.\n\n**Table functions:** `tokens(map)` (the \
                 full decoded mapping table), `sources(map)` (original files + inline-content \
                 flag), `meta(map)` (version, file, source_root, counts).\n\n**Table-in-out:** \
                 `lookup_all((SELECT id, line, col FROM frames), map := …, id := 'id')` — the \
                 bulk de-minify entry point, with `id` passthrough so results join back to the \
                 originating error.\n\n`lookup` returns the **nearest preceding** mapping (standard \
                 source-map semantics), so an exact `(line, col)` match isn't required. A \
                 malformed/empty map yields NULL resolution per row, never a query failure."
                    .to_string(),
            ),
            (
                "vgi.agent_test_tasks".to_string(),
                crate::meta::agent_test_tasks_json(&[
                    (
                        "count_mappings",
                        "I have a source map as JSON text: \
                         {\"version\":3,\"sources\":[\"a.js\"],\"names\":[],\"mappings\":\"AAAA,CAAC\"}. \
                         How many decoded mappings (tokens) does it contain? Return one column \
                         named n.",
                        "SELECT count(*) AS n FROM sourcemap.main.tokens(\
                         '{\"version\":3,\"sources\":[\"a.js\"],\"names\":[],\"mappings\":\"AAAA,CAAC\"}')",
                    ),
                    (
                        "resolve_origin",
                        "Using the source map JSON \
                         {\"version\":3,\"sources\":[\"index.js\"],\"names\":[],\"mappings\":\"AAAA\"}, \
                         resolve the minified position at generated line 0, column 0 and return \
                         the original source file as a single column named source.",
                        "SELECT (sourcemap.main.lookup(\
                         '{\"version\":3,\"sources\":[\"index.js\"],\"names\":[],\"mappings\":\"AAAA\"}', \
                         0, 0)).source AS source",
                    ),
                    (
                        "count_sources",
                        "How many original source files does this source map reference? Map JSON: \
                         {\"version\":3,\"sources\":[\"a.js\",\"b.js\"],\"names\":[],\"mappings\":\"AAAA\"}. \
                         Return one column named n_sources.",
                        "SELECT n_sources FROM sourcemap.main.meta(\
                         '{\"version\":3,\"sources\":[\"a.js\",\"b.js\"],\"names\":[],\"mappings\":\"AAAA\"}')",
                    ),
                    (
                        "bulk_resolve_join",
                        "I have one minified error frame with id 1 at generated line 0, column 0. \
                         Resolve it against the map \
                         {\"version\":3,\"sources\":[\"index.js\"],\"names\":[],\"mappings\":\"AAAA\"} \
                         and return the id and the resolved original source, as columns id and \
                         source.",
                        "SELECT r.id, r.source FROM sourcemap.main.lookup_all(\
                         (SELECT * FROM (VALUES (1, 0, 0)) AS f(id, line, col)), \
                         map := '{\"version\":3,\"sources\":[\"index.js\"],\"names\":[],\"mappings\":\"AAAA\"}', \
                         id := 'id') AS r",
                    ),
                    (
                        "worker_version",
                        "What version of the sourcemap worker is currently running? Return a \
                         single row with one column named version.",
                        "SELECT sourcemap.main.sourcemap_version() AS version",
                    ),
                ]),
            ),
            ("vgi.author".to_string(), "Query.Farm".to_string()),
            (
                "vgi.copyright".to_string(),
                "Copyright 2026 Query Farm LLC - https://query.farm".to_string(),
            ),
            ("vgi.license".to_string(), "MIT".to_string()),
            (
                "vgi.support_contact".to_string(),
                "https://github.com/Query-farm/vgi-sourcemap/issues".to_string(),
            ),
            (
                "vgi.support_policy_url".to_string(),
                "https://github.com/Query-farm/vgi-sourcemap/blob/main/README.md".to_string(),
            ),
        ],
        source_url: Some("https://github.com/Query-farm/vgi-sourcemap".to_string()),
        schemas: vec![CatSchema {
            name: "main".to_string(),
            comment: Some(
                "Source Map v3 decoding and minified-position resolution functions.".to_string(),
            ),
            tags: vec![
                ("vgi.title".to_string(), "Sourcemap — main".to_string()),
                (
                    "vgi.keywords".to_string(),
                    crate::meta::keywords_json(
                        "source map, tokens, lookup, lookup_all, sources, source_content, meta, \
                         de-minify, resolve, Base64 VLQ, mappings, stack frame, RUM",
                    ),
                ),
                // VGI123 classifying tags (bare keys) for faceting.
                ("domain".to_string(), "web-observability".to_string()),
                ("category".to_string(), "decoding-and-resolution".to_string()),
                ("topic".to_string(), "source-maps".to_string()),
                (
                    "vgi.doc_llm".to_string(),
                    "Functions for JavaScript Source Map v3: decode a map into its token table \
                     (tokens), resolve one minified position (lookup → STRUCT), bulk-resolve a \
                     column of frames with id passthrough (lookup_all), list original sources \
                     (sources) and fetch embedded source text (source_content), and report \
                     map metadata (meta). The map argument is a .map path, raw JSON text, or BLOB."
                        .to_string(),
                ),
                (
                    "vgi.doc_md".to_string(),
                    "The single schema for the `sourcemap` worker — the catalog name matches the \
                     `ATTACH` name, so qualify calls as `sourcemap.main.<fn>(...)`. It holds the \
                     `lookup` (→ STRUCT) and `source_content` (→ VARCHAR) scalars, the \
                     `sourcemap_version` scalar, the `tokens` / `sources` / `meta` table \
                     functions, and the `lookup_all` table-in-out function (bulk frame resolution \
                     with id passthrough). All line/column numbers are 0-indexed; `lookup` uses \
                     nearest-preceding semantics."
                        .to_string(),
                ),
                // VGI506 representative example queries (run against inline map text so they
                // execute without a .map file present).
                (
                    "vgi.example_queries".to_string(),
                    format!(
                        "SELECT * FROM sourcemap.main.tokens('{m}');\n\
                         SELECT sourcemap.main.lookup('{m}', 0, 0);\n\
                         SELECT * FROM sourcemap.main.sources('{m}');\n\
                         SELECT * FROM sourcemap.main.meta('{m}');\n\
                         SELECT sourcemap.main.sourcemap_version();",
                        m = SAMPLE_MAP
                    ),
                ),
            ],
            views: Vec::new(),
            macros: Vec::new(),
            tables: Vec::new(),
        }],
        ..Default::default()
    }
}

fn main() {
    // Logs MUST go to stderr — stdout is the Arrow-IPC channel.
    let _ = env_logger::Builder::from_env(env_logger::Env::default().filter_or("VGI_LOG", "info"))
        .format_timestamp_millis()
        .try_init();

    // The catalog name DuckDB sees in `ATTACH 'sourcemap' (TYPE vgi, …)`. Default
    // to `sourcemap`, but honor an explicit override so a test harness can rename it.
    if std::env::var_os("VGI_WORKER_CATALOG_NAME").is_none() {
        std::env::set_var("VGI_WORKER_CATALOG_NAME", "sourcemap");
    }
    let catalog_name =
        std::env::var("VGI_WORKER_CATALOG_NAME").unwrap_or_else(|_| "sourcemap".to_string());

    let mut worker = Worker::new();
    scalar::register(&mut worker);
    table::register(&mut worker);
    table_in_out::register(&mut worker);
    worker.set_catalog(catalog_metadata(&catalog_name));
    worker.run();
}
