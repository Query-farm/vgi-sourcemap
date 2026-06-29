# ci/ — integration test harness

`run-integration.sh` runs this repo's sqllogictest suite (`test/sql/*.test`)
against the built `sourcemap-worker` over the VGI protocol, using a prebuilt
standalone `haybarn-unittest` and the **signed community `vgi` extension** — no
DuckDB C++ build from source.

It is parameterized by `TRANSPORT` (`subprocess` | `http` | `unix`), exercising
the SAME suite over each transport the vgi extension supports; only the
`LOCATION` the `.test` files `ATTACH` (`VGI_SOURCEMAP_WORKER`) changes. The
fixture maps in `data/` are staged next to the runner so the path-mode tests
(`tokens('data/basic.js.map')`) resolve.

`preprocess-require.awk` rewrites `require <ext>` / `LOAD vgi;` lines into
explicit signed `INSTALL … ; LOAD … ;` so the standalone runner (which links no
extensions) can load `vgi` from the community channel — and, on the `http` leg,
`httpfs` from core (the vgi extension's HTTP client needs it; a missing httpfs
otherwise *silently auto-skips* the suite).

`check-version.sh <tag>` asserts the `[workspace.package]` version in
`Cargo.toml` matches a release tag; the release workflow runs it before
publishing.

Local one-liner (subprocess):

```sh
cargo build --release
HAYBARN_UNITTEST="$(command -v haybarn-unittest)" \
  WORKER_BIN="$PWD/target/release/sourcemap-worker" \
  ci/run-integration.sh
```
