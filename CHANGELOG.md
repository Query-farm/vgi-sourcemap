# Changelog

All notable changes to `vgi-sourcemap` are documented here.

## 0.4.2 (2026-11-16)

- VGI Rust SDK `vgi` 0.51.0 → 0.51.1 (`vgi-rpc` stays 0.32.4).
- SQL tests now run through the starter's `ci/run-integration.sh` over stdio, launch and http; `make check` runs everything CI runs.
- CI image test installs the pinned `haybarn-unittest`; new `make lint-explain` metadata gate (vgi-lint at fail-on=info).

## [0.4.1] - 2026-11-14

### Changed

- Docs link to upstream sources: the catalog docs now link the ECMA-426 source map
  specification and the Sentry `sourcemap` crate the worker wraps (vgi-lint VGI183).
