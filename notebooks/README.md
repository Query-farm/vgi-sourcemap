# Source Maps

[01 · A resolved frame still needs context](01_a_resolved_frame_still_needs_context.ipynb)
decodes an authored source map, resolves fifteen synthetic stack frames, and separates
nearest-preceding matches from an explicit reporting policy. SVG charts retain large
gaps and reconcile the full input population. The worker's built-in example introduces
zero-based coordinates; wrapper metadata is checked against decoded map contents.

Open the notebook's **Open in Colab** link, or run locally with [uv](https://docs.astral.sh/uv/):

```sh
cd notebooks
uv sync --locked
uv run --locked vgi-notebooks
```

A notebook downloaded on its own retrieves the verified authored fixture and plotting
companion through the shared package. Colab installs only the base package and retains
compatible preloaded libraries; the local lock includes the runner. Select
`notebooks/.venv/bin/python` in a notebook editor. Restart and run all after editing.

The visible connection pins the native worker release:

```sql
INSTALL vgi FROM community;
LOAD vgi;
ATTACH 'sourcemap' (TYPE vgi,
  LOCATION 'github-auto://Query-farm/vgi-sourcemap@v0.4.0');
```

Initial setup needs network access; the analysis then uses synthetic local files.
No worker source build, credentials or live telemetry is needed. Execution was tested
on macOS arm64. A preloaded-kernel compatibility run checks the one-file bootstrap;
hosted Colab itself was not manually executed. GitHub displays saved tables and SVGs.

## Experiment

```sh
uv run --locked vgi-notebooks --parameters '{"MAX_COLUMN_GAP":10}' \
  --output-dir artifacts/gap-10
```

Raising the gap from four to ten includes eight rather than six of fifteen frames.
Raw resolved frames stay at twelve; F04 and F11 alone change policy category.
The map, decoded tokens and raw results remain fixed. The separate output directory
preserves the saved default, and both chart axes retain their default scale.

## Authored fixture

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

The notebook meaningfully demonstrates all **7/7 public SQL surfaces** in native
v0.4.0. Current source has the same interface, established by a retained registration
and metadata/documentation-only diff review; execution used the release. Catalogs,
reviewed mappings and successful execution receipts live in [coverage.json](coverage.json)
and [coverage/](coverage/). Thirteen SQL checks and two chart population checks protect
the interpretation. Defaults, the parameter experiment, portable setup and figures
are verified before publication.

The shared [notebook standard](https://github.com/Query-farm/vgi-example-notebooks/blob/main/docs/standard.md)
owns setup, execution, branding and SVG publication.
