//! Shared helpers for the per-object discovery/description metadata that the
//! `vgi-lint` strict profile expects on **every** function and table.
//!
//! Each function/table surfaces these in its `FunctionMetadata.tags`:
//! - `vgi.title` (VGI124)        — human-friendly display name
//! - `vgi.doc_llm` (VGI112)      — concise prose aimed at LLMs
//! - `vgi.doc_md` (VGI113)       — short Markdown description
//! - `vgi.keywords` (VGI126/VGI138) — a JSON array of search terms/synonyms
//!
//! Per-object `vgi.source_url` is intentionally NOT emitted here: it belongs on
//! the catalog object only (VGI139), where the catalog's `source_url` field
//! already points at the repo.

/// Encode comma-separated keywords as the JSON array of strings `vgi.keywords`
/// requires (VGI138): e.g. `["source map","decode","lookup"]`.
pub fn keywords_json(keywords: &str) -> String {
    let items: Vec<String> = keywords
        .split(',')
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(|k| {
            let escaped = k.replace('\\', "\\\\").replace('"', "\\\"");
            format!("\"{escaped}\"")
        })
        .collect();
    format!("[{}]", items.join(","))
}

/// Build the `vgi.agent_test_tasks` JSON value: a fixed suite of analyst tasks
/// that `vgi-lint simulate` runs. Each `(name, prompt)` pair becomes a task
/// object shown to the simulated analyst. The graders (`reference_sql`, the
/// canonical solution used to grade) are private and live in the
/// `vgi-agent-tests.yaml` sidecar at the repo root, keyed by task name (VGI416).
pub fn agent_test_tasks_json(tasks: &[(&str, &str)]) -> String {
    fn esc(s: &str) -> String {
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    }
    let items: Vec<String> = tasks
        .iter()
        .map(|(name, prompt)| {
            format!(
                "{{\"name\":\"{}\",\"prompt\":\"{}\"}}",
                esc(name),
                esc(prompt)
            )
        })
        .collect();
    format!("[{}]", items.join(","))
}

/// Build a `vgi.example_queries` value: a JSON list of `{"description","sql"}`
/// objects (VGI515). This is the carrier that preserves each example's
/// description — the native `duckdb_functions().examples` column drops it — so
/// every function that ships a `FunctionExample` also emits a byte-identical
/// described-example tag here.
pub fn example_queries_json(examples: &[(&str, &str)]) -> String {
    fn esc(s: &str) -> String {
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    }
    let items: Vec<String> = examples
        .iter()
        .map(|(description, sql)| {
            format!(
                "{{\"description\":\"{}\",\"sql\":\"{}\"}}",
                esc(description),
                esc(sql)
            )
        })
        .collect();
    format!("[{}]", items.join(","))
}

/// Build the standard per-object discovery/description tags.
///
/// `category` must name one of the schema's `vgi.categories` entries (VGI413);
/// it is emitted as the object's `vgi.category` tag so the linter and UIs can
/// group functions under their schema-declared category.
pub fn object_tags(
    title: &str,
    description_llm: &str,
    description_md: &str,
    keywords: &str,
    category: &str,
) -> Vec<(String, String)> {
    vec![
        ("vgi.title".to_string(), title.to_string()),
        ("vgi.doc_llm".to_string(), description_llm.to_string()),
        ("vgi.doc_md".to_string(), description_md.to_string()),
        ("vgi.keywords".to_string(), keywords_json(keywords)),
        ("vgi.category".to_string(), category.to_string()),
    ]
}
