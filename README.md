<div align="center">
  <img src="https://raw.githubusercontent.com/Query-farm/vgi-lint-check/main/assets/vgi-logo.png" alt="Vector Gateway Interface" width="320">
</div>

# vgi-sourcemap

A [VGI](https://query.farm) worker that decodes **JavaScript Source Map v3**
(`.map`) files — the Base64-VLQ `mappings` grammar — into rows, and resolves
minified `(line, col)` positions back to original `source:line:col:name`, all in
DuckDB SQL.

For front-end RUM / error-tracking teams that store minified stack frames and
want to **de-minify them in bulk in SQL**. Nothing else in DuckDB reads source
maps — `read_json` gives you the wrapper object but not the decoded `mappings`
VLQ. This turns a column of `(error_url, line, col)` frames into resolved original
source locations with one JOIN.

The heavy lifting (Base64-VLQ decode, index maps via `sections`, inline
`sourcesContent`, the nearest-preceding lookup) is done by Sentry's
[`sourcemap`](https://crates.io/crates/sourcemap) crate (BSD-3-Clause); this
worker is a thin Apache-Arrow adapter over it.

## Install & attach

The worker is a standalone binary DuckDB launches and talks to over Arrow IPC.

```sql
INSTALL vgi FROM community; LOAD vgi;
ATTACH 'sourcemap' (TYPE vgi, LOCATION '/path/to/sourcemap-worker');
SET search_path = 'sourcemap.main';
```

Build the binary with `cargo build --release` (it lands at
`target/release/sourcemap-worker`).

## The `map` argument

Every function takes its source map the same way — a single overloaded `map`
argument that is one of:

- a **path** to a `.map` file on the worker's filesystem
  (`tokens('/dist/app.min.js.map')`),
- the raw map **JSON text** (`tokens('{"version":3,…}')`, e.g. a `data:` URI
  payload or a column of fetched map bodies), or
- the map **bytes** (a `BLOB`).

A `VARCHAR` is treated as inline JSON when it begins with `{` (after an optional
`)]}'` XSSI guard); otherwise it is read as a path.

> All line and column numbers — both the generated positions you pass in and the
> original positions you get back — are **0-indexed**, exactly as the Source Map
> v3 spec defines them. Browser engines usually report 1-indexed lines, so
> subtract 1 before calling and add 1 to the result.

## SQL surface

| Area | SQL | Returns |
| --- | --- | --- |
| **All tokens** | `tokens(map)` | table: `gen_line, gen_col, src_id, source, src_line, src_col, name` |
| **Resolve one position** | `lookup(map, gen_line, gen_col)` | `STRUCT(source, src_line, src_col, name)` |
| **Resolve a column** | `lookup_all((SELECT id, line, col FROM frames), map := …, id := 'id')` | table: `id` passthrough + `source, src_line, src_col, name` |
| **Sources** | `sources(map)` | table: `src_id, source, has_content` |
| **Inline content** | `source_content(map, src_id)` | `VARCHAR` |
| **Metadata** | `meta(map)` | table: `version, file, source_root, n_sources, n_names, n_mappings` |

The running worker build is published as the catalog's `implementation_version`
(read it from `vgi_catalogs()`), so there is no separate version function.

**Conventions**

- **`lookup` returns the nearest preceding mapping** (standard source-map
  semantics) so an exact `(line, col)` match isn't required — that's how real
  stack frames resolve.
- **`lookup_all`** is the practical entry point: stream a column of minified
  frames → resolved rows, with `id` passthrough to join back to the originating
  error.
- **Index maps** (`sections`) and inline `sourcesContent` are supported;
  data-URI inline maps decode via the text/bytes mode.
- **Per-row safety:** a malformed/empty map yields NULL resolution, not a query
  failure.

## Examples

```sql
-- Explode an entire map into its token table.
SELECT * FROM sourcemap.main.tokens('/dist/app.min.js.map') LIMIT 10;

-- Resolve one minified frame to its original location (nearest-preceding).
SELECT (sourcemap.main.lookup('/dist/app.min.js.map', 0, 12)).*;

-- Bulk de-minify a column of production error frames, joining back by id.
SELECT e.id, r.source, r.src_line, r.name
FROM errors e
JOIN sourcemap.main.lookup_all(
       (SELECT id, gen_line AS line, gen_col AS col FROM errors),
       map := '/dist/app.min.js.map',
       id := 'id') AS r ON r.id = e.id;

-- What original files does this bundle come from, and which have inline content?
SELECT * FROM sourcemap.main.sources('/dist/app.min.js.map');

-- Fetch the embedded original source text for one source.
SELECT sourcemap.main.source_content('/dist/app.min.js.map', 0);

-- Size / sanity-check a map before decoding it in full.
SELECT * FROM sourcemap.main.meta('/dist/app.min.js.map');
```

Passing the map as inline JSON text works identically (handy when the map bodies
live in a table column):

```sql
SELECT count(*) FROM sourcemap.main.tokens(
  '{"version":3,"sources":["index.js"],"names":[],"mappings":"AAAA,CAAC"}');
```

## Scope / non-goals

**v1:** decode v3 maps (regular + index/`sections`) + resolve positions (single
and bulk). **Non-goals:** fetching maps over HTTP from a `sourceMappingURL` (the
caller supplies the map text/path/bytes); generating source maps; symbol-server
hosting.

## Development

```sh
cargo test --workspace --all-features    # unit + golden-fixture + proptest fuzz
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
cargo build --release

# End-to-end SQLLogic suite (needs haybarn-unittest + the community vgi extension):
HAYBARN_UNITTEST="$(command -v haybarn-unittest)" \
  WORKER_BIN="$PWD/target/release/sourcemap-worker" ci/run-integration.sh
```

See [`CLAUDE.md`](CLAUDE.md) for the build/layout notes and sharp edges.

## License

MIT — Copyright 2026 Query Farm LLC · https://query.farm. The bundled
`sourcemap` crate is BSD-3-Clause.
