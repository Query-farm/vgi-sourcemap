# VGI SourceMap notebooks

[01 · A resolved frame still needs context](01_a_resolved_frame_still_needs_context.ipynb) decodes an authored source map, resolves synthetic stack frames in bulk, and separates raw nearest-preceding matches from an explicit reporting policy. Charts retain large gaps and reconcile all input frames.

## Run

Install [uv](https://docs.astral.sh/uv/). From the repository root:

```sh
uv sync --project notebooks --frozen
uv run --project notebooks --frozen python notebooks/execute.py
```

The complete `notebooks/` directory also runs separately from the worker checkout:

```sh
cd notebooks
uv sync --frozen
uv run --frozen python execute.py
```

Initial setup needs network access for the locked Python environment, VGI community extension and pinned worker release. Subsequent analysis reads only synthetic local files. No Rust toolchain, credentials, live telemetry or external dataset is required. Native release archives cover macOS Apple silicon, Linux x86-64/arm64 and Windows x86-64; this notebook was executed on macOS Apple silicon.

In VS Code select `notebooks/.venv/bin/python` and use `notebooks/` as the kernel working directory. Other Jupyter editors can register the environment:

```sh
uv run --project notebooks --frozen python -m ipykernel install --user --name vgi-sourcemap-notebooks --display-name 'Python (vgi-sourcemap notebooks)'
```

Restart and run all after editing parameters. Connection and analytical SQL remain expanded; setup, plotting, assertions and cleanup source are folded. Saved charts are embedded in the notebook.

## Experiment

```sh
uv run --project notebooks --frozen python notebooks/execute.py \
  --parameters '{"MAX_COLUMN_GAP":10}' \
  --output-dir notebooks/validation/gap-10
```

The default gap is four generated columns. Raising it to ten changes only the reporting policy; it does not improve the map or alter raw resolution. Reset the parameter to four for the saved default.

The runner requires a separate output directory for overrides, rejects unknown parameter names, and saves only successful executions. Failed runs leave prior saved notebooks untouched. Experiment copies include relative assets, fixture, runner and environment files. Validation logs, extracted charts and experiment copies are ignored by Git.

## Portable startup and fixture provenance

The visible literal `ATTACH` location is `github-auto://Query-farm/vgi-sourcemap@v0.4.0`. VGI downloads the platform archive, checks its checksum and caches it. Haybarn is pinned to 2.0.0rc11 in the isolated notebook lock. To upgrade, change the release tag explicitly and rerun both settings.

`fixtures/app.min.js.map` is authored Source Map v3 JSON with five mapping tokens. Its exact Base64-VLQ mapping string is `EAAAA,QACEC,YCDFC;IACEC,cDCAC`. The decoded zero-based coordinates are checked against independent SQL values:

| Generated line | Column | Source | Original line | Column | Name |
| --- | --- | --- | --- | --- | --- |
| 0 | 2 | checkout.js | 0 | 0 | checkout |
| 0 | 10 | checkout.js | 1 | 2 | validate |
| 0 | 22 | price.js | 0 | 0 | price |
| 1 | 4 | price.js | 1 | 2 | round |
| 1 | 18 | checkout.js | 2 | 2 | submit |

The source root is `/src`; the worker joins it to the two paths. Short original snippets are embedded in `sourcesContent`. There is no corresponding deployed minified bundle: this fixture demonstrates lookup semantics, not a build-integrity check. Fifteen synthetic frame rows are authored visibly in SQL. Both reported lines and columns are explicitly one based in this fixture; real producers may use different conventions. The worker receives normalized zero-based coordinates. One missing line and one invalid zero line remain separate from valid-but-unresolved positions.

## Interpretation and limits

The worker's nearest-preceding lookup can return a token from an earlier generated line or far behind the requested column. It returns stored original coordinates, without interpolation. The notebook's SQL audit finds the preceding token and compares its source coordinates with the real worker result. A same-line column gap is a distance in generated coordinates, not a probability or confidence score.

The illustrative reporting policy requires a source, the same generated line, and a gap at or below the threshold. Real maps can legitimately be sparse; excluded matches are not necessarily wrong. A close match is not proof that the right build map was supplied. All original rows stay in the audit, so the narrower chart does not hide unresolved or invalid frames. Counts are frame rows, not exception or user counts.

Per-row scalar lookup tolerates malformed/missing maps and returns NULL, as it also does before the first token. Whole-map functions such as `tokens()` surface map errors instead. Other maps can contain tokens with no source information; every token in this fixture has a source. Original content can be absent in real maps and must not be invented. The worker does not retrieve map URLs.

Validation covers both parameter settings, execution from a detached notebook directory, four rendered charts, twelve SQL checks plus chart reconciliation, and three runner guards. The default run took 7.8 seconds; the detached ten-column experiment took 4.1 seconds and includes 8 rather than 6 of 15 frames. Raw resolved frames stay at 12; F04 and F11 alone change policy category. Source validation passed 23 Rust tests, 65 stdio / 71 HTTP / 65 Unix SQL assertions (three cases each), Clippy, rustfmt, warning-clean rustdoc, locked release build, and the configured cargo-audit gate. Metadata scored 100/100 L2 with zero findings. Notebook execution uses the release; source validation builds only the worker and preserves source, tests, fixtures and Cargo files.

Official Query.Farm Strata Sun and VGI artwork is copied unchanged from the approved assets, with Query.Farm branding at the top and bottom.
