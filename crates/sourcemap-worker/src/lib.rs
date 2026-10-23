//! The `sourcemap` VGI worker (library).
//!
//! Function registration and catalog metadata live here so both entrypoints
//! share them verbatim: `main.rs` (the native binary, stdio/HTTP transport) and
//! the `sourcemap-wasm` crate (the browser build, which serves the same `Worker`
//! over a SharedArrayBuffer byte channel instead).
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
// Public so integration tests (`tests/`) and downstream consumers can exercise
// the pure Source Map v3 decode/resolution engine without Arrow or RPC.
pub mod smap;
mod table;
mod table_in_out;

use vgi::catalog::{CatSchema, CatView, CatalogModel};
use vgi::Worker;

/// Worker build version, published as the catalog's `implementation_version` (an
/// agent reads it from `vgi_catalogs()` without spending a query, and it can't
/// drift from the running build).
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
                 (line, col) positions back to their original source:line:col:name. A source map \
                 encodes the mapping between a minified bundle and its original files as a \
                 Base64-VLQ 'mappings' string; ordinary JSON readers expose the wrapper object but \
                 cannot decode that grammar, so this worker does the decode and exposes both the \
                 raw mapping rows and resolved original positions as queryable results. A map is \
                 supplied as a path to a .map file, the raw map JSON text, or map `BLOB` bytes; \
                 index maps (sections) and inline sourcesContent are supported. All line/column \
                 numbers are 0-indexed, and position resolution uses nearest-preceding semantics \
                 (the greatest mapping <= the requested position), so an exact match is not \
                 required. Per-row resolution is tolerant — a NULL, malformed, or empty map yields \
                 NULL rather than aborting the scan — which makes it suited to de-minifying whole \
                 columns of production RUM / error-tracking stack frames in bulk with one JOIN."
                    .to_string(),
            ),
            (
                "vgi.doc_md".to_string(),
                "# sourcemap\n\nDecode **JavaScript Source Map v3** (`.map`) files and resolve \
                 minified `(line, col)` positions back to original `source:line:col:name`, \
                 directly in DuckDB SQL. Nothing else in DuckDB reads source maps — `read_json` \
                 gives you the wrapper object but not the decoded Base64-VLQ `mappings` grammar. \
                 This worker performs that decode and exposes both the raw mapping rows and \
                 resolved original positions, so a column of production RUM / error-tracking \
                 frames can be de-minified in bulk with a single JOIN.\n\nThe heavy lifting \
                 (Base64-VLQ decode, index maps via `sections`, inline `sourcesContent`, the \
                 nearest-preceding lookup) is done by the Sentry `sourcemap` crate; this worker is \
                 a thin Arrow adapter over it. A map is supplied as a **path** to a `.map` file, \
                 the raw map **JSON text** (e.g. from `read_text(...)`), or map `BLOB` bytes — \
                 so data-URI inline maps decode via the text/bytes mode.\n\n## Semantics\n\n- All \
                 line/column numbers are **0-indexed** (Source Map v3); browser stack frames are \
                 usually 1-indexed, so subtract one before resolving.\n- Position resolution is \
                 **nearest-preceding** (the greatest mapping `<=` the requested position), so an \
                 exact `(line, col)` match isn't required.\n- Per-row resolution is **tolerant**: \
                 a NULL, malformed, or empty map — or a position with no match — yields NULL \
                 rather than failing the query, so one bad frame never aborts a scan.\n\n## When \
                 to use\n\nReach for it whenever you have minified positions (deployed bundle \
                 line/column) and the corresponding source map, and you want the original file, \
                 line, column, and symbol name back — one frame at a time or a whole error table \
                 at once."
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
                    ),
                    (
                        "resolve_origin",
                        "Using the source map JSON \
                         {\"version\":3,\"sources\":[\"index.js\"],\"names\":[],\"mappings\":\"AAAA\"}, \
                         resolve the minified position at generated line 0, column 0 and return \
                         the original source file as a single column named source.",
                    ),
                    (
                        "count_sources",
                        "How many original source files does this source map reference? Map JSON: \
                         {\"version\":3,\"sources\":[\"a.js\",\"b.js\"],\"names\":[],\"mappings\":\"AAAA\"}. \
                         Return one column named n_sources.",
                    ),
                    (
                        "bulk_resolve_join",
                        "I have one minified error frame with id 1 at generated line 0, column 0. \
                         Resolve it against the map \
                         {\"version\":3,\"sources\":[\"index.js\"],\"names\":[],\"mappings\":\"AAAA\"} \
                         and return the id and the resolved original source, as columns id and \
                         source.",
                    ),
                    (
                        "inline_source_text",
                        "A source map can embed its original files inline via sourcesContent. \
                         For the map JSON \
                         {\"version\":3,\"sources\":[\"a.js\"],\"sourcesContent\":[\"const x = 1;\"],\
                         \"names\":[],\"mappings\":\"AAAA\"}, return the embedded original source \
                         text for source id 0 as a single column named content.",
                    ),
                    (
                        "sources_with_inline_content",
                        "For the source map \
                         {\"version\":3,\"sources\":[\"a.js\",\"b.js\"],\
                         \"sourcesContent\":[\"const x=1;\",null],\"names\":[],\"mappings\":\"AAAA\"}, \
                         how many of its original source files embed inline text? Return the count \
                         as a single column named n.",
                    ),
                    (
                        "preview_example_tokens",
                        "This worker ships a browsable example view of decoded source-map tokens. \
                         How many token (mapping) rows does that built-in example contain? Return \
                         the count as a single column named n.",
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
            path: Vec::new(),
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
                    "The single schema for JavaScript Source Map v3 work: decoding a map's \
                     Base64-VLQ mappings into rows and resolving minified (line, col) positions \
                     back to original source locations. A map is supplied as a .map path, raw \
                     JSON text, or `BLOB`. All line/column numbers are 0-indexed and position \
                     resolution is nearest-preceding, so an exact match is not required. Functions \
                     are grouped into resolution and inspection categories."
                        .to_string(),
                ),
                (
                    "vgi.doc_md".to_string(),
                    "## main\n\nThe single schema for the `sourcemap` worker. The catalog name \
                     matches the `ATTACH` name, so qualify calls as `sourcemap.main.<fn>(...)`.\n\n\
                     It covers decoding a Source Map v3 map into its mapping rows and resolving \
                     minified `(line, col)` positions back to original `source:line:col:name`.\n\n\
                     ## Semantics\n\n- All line/column numbers are **0-indexed**.\n- Position \
                     resolution is **nearest-preceding** (the greatest mapping `<=` the requested \
                     position), so an exact match isn't required.\n- Per-row resolution is \
                     tolerant: a NULL, malformed, or empty map yields NULL rather than failing the \
                     query.\n\n## Categories\n\nFunctions are organized under the schema's \
                     `vgi.categories`: **Resolution** and **Inspection**."
                        .to_string(),
                ),
                // VGI413 category registry: the ordered set of categories every object's
                // `vgi.category` tag must name. Descriptions state the category's purpose (not an
                // object inventory).
                (
                    "vgi.categories".to_string(),
                    "[\
                     {\"name\":\"Resolution\",\
                     \"description\":\"Resolve minified (line, col) positions back to their \
                     original source locations, one frame or a whole column at a time.\"},\
                     {\"name\":\"Inspection\",\
                     \"description\":\"Decode a source map and inspect its contents — the decoded \
                     mappings, original files, embedded source text, and map-level metadata.\"}\
                     ]"
                        .to_string(),
                ),
                // VGI506/VGI515 representative example queries — a described-example
                // JSON list (each carries a human-readable description). They run
                // against inline map text so they execute without a .map file present,
                // and project explicit columns (never a bare `SELECT *`).
                (
                    "vgi.example_queries".to_string(),
                    crate::meta::example_queries_json(&[
                        (
                            "Decode a source map into its full token (mapping) table.",
                            &format!(
                                "SELECT gen_line, gen_col, source FROM sourcemap.main.tokens('{SAMPLE_MAP}')"
                            ),
                        ),
                        (
                            "Resolve a minified (line, col) position to its original source location.",
                            &format!("SELECT sourcemap.main.lookup('{SAMPLE_MAP}', 0, 0)"),
                        ),
                        (
                            "List the original source files a map references.",
                            &format!(
                                "SELECT src_id, source, has_content FROM sourcemap.main.sources('{SAMPLE_MAP}')"
                            ),
                        ),
                        (
                            "Read a map's version and mapping counts.",
                            &format!(
                                "SELECT version, n_sources, n_mappings FROM sourcemap.main.meta('{SAMPLE_MAP}')"
                            ),
                        ),
                    ]),
                ),
            ],
            // NOT a self-cataloguing function registry (VGI327): a browsable
            // *data* view (VGI146). Selecting it decodes a tiny built-in sample
            // map into its token rows, so an agent can `SELECT *` and see the exact
            // shape `tokens(map)` produces — column names, types, 0-indexing — with
            // no `.map` file, argument, or network. It is the worker's "here's what
            // the output looks like" entry point.
            views: vec![CatView {
                name: "example_tokens".to_string(),
                definition: format!(
                    "SELECT gen_line, gen_col, src_id, source, src_line, src_col, name \
                     FROM sourcemap.main.tokens('{SAMPLE_MAP}')"
                ),
                comment: Some(
                    "A worked example: the decoded token (mapping) rows of a tiny built-in \
                     sample source map, so you can see the shape of tokens(map) output without \
                     supplying a map."
                        .to_string(),
                ),
                tags: vec![
                    (
                        "vgi.title".to_string(),
                        "Sourcemap — Example Decoded Tokens".to_string(),
                    ),
                    ("vgi.category".to_string(), "Inspection".to_string()),
                    ("domain".to_string(), "web-observability".to_string()),
                    (
                        "vgi.keywords".to_string(),
                        crate::meta::keywords_json(
                            "example, sample, demo, tokens, decoded mappings, output shape, \
                             preview, getting started, gen_line, gen_col, source",
                        ),
                    ),
                    (
                        "vgi.doc_llm".to_string(),
                        "A zero-argument, browsable preview of the worker's output: the decoded \
                         token rows of a tiny built-in sample Source Map v3. Select it to see \
                         exactly what `tokens(map)` returns — the columns (gen_line, gen_col, \
                         src_id, source, src_line, src_col, name), their types, and the 0-indexed \
                         convention — before constructing a real `map` argument. It is a fixed \
                         demo slice, not live data: to decode your own map, call \
                         `tokens(<your map>)` with a .map path, raw JSON text, or map `BLOB` bytes."
                            .to_string(),
                    ),
                    (
                        "vgi.doc_md".to_string(),
                        "# example_tokens\n\nA browsable, credential-free **preview** of the \
                         worker's output. Selecting it decodes a tiny built-in sample source map \
                         into its token rows, so you can see the exact shape `tokens(map)` returns \
                         — the columns, their types, and the 0-indexed convention — with no `.map` \
                         file, argument, or network.\n\nColumns match `tokens(map)`: `gen_line`, \
                         `gen_col`, `src_id`, `source`, `src_line`, `src_col`, `name`.\n\nThis is a \
                         fixed demo slice. To decode your own map, call \
                         `tokens(<your map>)` with a `.map` path, raw JSON text, or map `BLOB` \
                         bytes."
                            .to_string(),
                    ),
                    (
                        "vgi.example_queries".to_string(),
                        crate::meta::example_queries_json(&[
                            (
                                "Preview the decoded token rows of the built-in sample map.",
                                "SELECT gen_line, gen_col, source FROM sourcemap.main.example_tokens",
                            ),
                            (
                                "Count how many decoded mappings the sample map contains.",
                                "SELECT count(*) AS n FROM sourcemap.main.example_tokens",
                            ),
                        ]),
                    ),
                ],
                column_comments: vec![
                    (
                        "gen_line".to_string(),
                        "0-indexed generated (minified) line of this mapping.".to_string(),
                    ),
                    (
                        "gen_col".to_string(),
                        "0-indexed generated (minified) column of this mapping.".to_string(),
                    ),
                    (
                        "src_id".to_string(),
                        "0-based index into sources() of the original file, or NULL if the token \
                         has no source."
                            .to_string(),
                    ),
                    (
                        "source".to_string(),
                        "Original source file path this mapping points at, or NULL.".to_string(),
                    ),
                    (
                        "src_line".to_string(),
                        "0-indexed original source line number (0 = the first line), or NULL."
                            .to_string(),
                    ),
                    (
                        "src_col".to_string(),
                        "0-indexed original source column number (0 = the first column), or NULL."
                            .to_string(),
                    ),
                    (
                        "name".to_string(),
                        "Original symbol name for this mapping, or NULL if none.".to_string(),
                    ),
                ],
            }],
            macros: Vec::new(),
            tables: Vec::new(),
        }],
        // Publish the running build as the catalog's implementation_version (VGI328):
        // an agent reads it from `vgi_catalogs()` without spending a query, and it can
        // never drift from the binary — which is why there is no `sourcemap_version()`
        // scalar.
        implementation_version: Some(version().to_string()),
        ..Default::default()
    }
}

/// The catalog name DuckDB sees in `ATTACH 'sourcemap' (TYPE vgi, …)`. Defaults
/// to `sourcemap`, but honors an explicit override so a test harness can rename
/// it. Must match the ATTACH name.
pub fn catalog_name() -> String {
    if std::env::var_os("VGI_WORKER_CATALOG_NAME").is_none() {
        std::env::set_var("VGI_WORKER_CATALOG_NAME", "sourcemap");
    }
    std::env::var("VGI_WORKER_CATALOG_NAME").unwrap_or_else(|_| "sourcemap".to_string())
}

/// Build the fully-registered worker (scalars, table, table-in-out functions and
/// catalog metadata) without running it. Both entrypoints — the native binary and
/// the wasm browser build — call this and then drive the transport themselves.
pub fn build_worker() -> Worker {
    let name = catalog_name();
    let mut worker = Worker::new();
    scalar::register(&mut worker);
    table::register(&mut worker);
    table_in_out::register(&mut worker);
    worker.set_catalog(catalog_metadata(&name));
    worker
}
