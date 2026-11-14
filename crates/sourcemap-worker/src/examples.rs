//! Composed example queries: SQL that combines two or more of the worker's own
//! objects (`sources` + `source_content`, `lookup_all` + `sources` +
//! `source_content`, `tokens` + `sources`) in one realistic query.
//!
//! Each SQL is defined exactly once here and referenced from both the schema's
//! `vgi.example_queries` list and the example list of the object that drives the
//! query. They all run against the inline [`compose_map!`] text, so they bind and
//! execute offline with deterministic output.

/// A small but realistic inline v3 map for the composed examples: a two-line
/// bundle built from `src/index.js` (line 0) and `src/util.js` (line 1), plus a
/// tree-shaken `src/unused.js` that is listed in `sources` (with inline
/// `sourcesContent`) but has no mappings. Every source embeds its text and the
/// tokens carry symbol names, so source lines can be recovered from a frame.
///
/// A macro (not a `const`) so it can be spliced into `concat!` and keep every
/// example SQL a compile-time `&'static str`. It contains no `'`, so it is safe
/// inside a SQL string literal.
macro_rules! compose_map {
    () => {
        "{\"version\":3,\"file\":\"app.min.js\",\"sources\":[\"src/index.js\",\"src/util.js\",\"src/unused.js\"],\"sourcesContent\":[\"function main() {\\n  console.log(add(1, 2));\\n}\\nmain();\\n\",\"function add(a, b) {\\n  return a + b;\\n}\\n\",\"function unused() {\\n  return 0;\\n}\\n\"],\"names\":[\"main\",\"console\",\"log\",\"add\"],\"mappings\":\"AAAA,SAASA,OACPC,QAAQC,IAAIC,UAEdH;ACHA,SAASG,SACP\"}"
    };
}

/// The composed-example map as a plain `&str` (agent-task prompts, tests).
pub(crate) const COMPOSE_MAP: &str = compose_map!();

/// `sources` → `source_content`: every original file with the size of its
/// embedded source text.
pub(crate) const SOURCES_WITH_CONTENT_DESC: &str =
    "List each original file that embeds its source text, with that text's line and character \
     counts, by feeding each sources() src_id into source_content().";
pub(crate) const SOURCES_WITH_CONTENT_SQL: &str = concat!(
    "SELECT src_id, source, len(string_split(rtrim(content, chr(10)), chr(10))) AS n_lines, length(content) AS n_chars FROM (SELECT s.src_id, s.source, sourcemap.main.source_content('",
    compose_map!(),
    "', s.src_id) AS content FROM sourcemap.main.sources('",
    compose_map!(),
    "') AS s WHERE s.has_content) ORDER BY src_id"
);

/// `lookup_all` → `sources` → `source_content`: de-minify a batch of frames and
/// show the original line of code each frame points at.
pub(crate) const FRAMES_WITH_CODE_DESC: &str =
    "De-minify a batch of error frames with lookup_all, then join sources() and use \
     source_content() to show the original line of code each frame lands on (orig_line is \
     1-indexed).";
pub(crate) const FRAMES_WITH_CODE_SQL: &str = concat!(
    "WITH frames(id, line, col) AS (VALUES (1, 0, 30), (2, 1, 20), (3, 0, 40)) SELECT r.id, r.source, r.src_line + 1 AS orig_line, r.name, trim(string_split(sourcemap.main.source_content('",
    compose_map!(),
    "', s.src_id), chr(10))[r.src_line + 1]) AS code FROM sourcemap.main.lookup_all((SELECT id, line, col FROM frames), map := '",
    compose_map!(),
    "', id := 'id') AS r JOIN sourcemap.main.sources('",
    compose_map!(),
    "') AS s ON s.source = r.source ORDER BY r.id"
);

/// `tokens` → `sources`: per-file mapping coverage, keeping files that no
/// mapping points at (tree-shaken / dead code).
pub(crate) const MAPPINGS_PER_SOURCE_DESC: &str =
    "Count the decoded mappings (and distinct symbol names) pointing into each original file by \
     LEFT JOINing tokens() onto sources(), so a file with zero mappings (tree-shaken code) still \
     appears with a count of 0.";
pub(crate) const MAPPINGS_PER_SOURCE_SQL: &str = concat!(
    "SELECT s.source, count(t.gen_col) AS n_mappings, count(DISTINCT t.name) AS n_names FROM sourcemap.main.sources('",
    compose_map!(),
    "') AS s LEFT JOIN sourcemap.main.tokens('",
    compose_map!(),
    "') AS t ON t.src_id = s.src_id GROUP BY s.src_id, s.source ORDER BY s.src_id"
);

/// All composed examples, in the order the schema-level list shows them.
pub(crate) const COMPOSED: [(&str, &str); 3] = [
    (FRAMES_WITH_CODE_DESC, FRAMES_WITH_CODE_SQL),
    (SOURCES_WITH_CONTENT_DESC, SOURCES_WITH_CONTENT_SQL),
    (MAPPINGS_PER_SOURCE_DESC, MAPPINGS_PER_SOURCE_SQL),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::smap::Smap;

    /// The composed examples' descriptions rely on this exact shape: three
    /// sources (the last with no mappings), every source embedding its text,
    /// six mappings into index.js and three into util.js.
    #[test]
    fn compose_map_shape() {
        let m = Smap::parse(COMPOSE_MAP.as_bytes()).expect("compose map parses");
        let meta = m.meta();
        assert_eq!(meta.n_sources, 3);
        assert_eq!(meta.n_mappings, 9);
        let per_src = |id: u32| m.tokens().iter().filter(|t| t.src_id == Some(id)).count();
        assert_eq!((per_src(0), per_src(1), per_src(2)), (6, 3, 0));
        assert!(m.sources().iter().all(|s| s.has_content));
    }

    /// Every composed SQL is catalog-qualified, ordered, and free of `SELECT *`.
    #[test]
    fn composed_sql_is_lint_shaped() {
        for (desc, sql) in COMPOSED {
            assert!(!desc.is_empty());
            assert!(sql.contains("ORDER BY"), "{sql}");
            assert!(!sql.contains("SELECT *"), "{sql}");
            assert!(!sql.contains('\n'), "{sql}");
        }
    }
}
