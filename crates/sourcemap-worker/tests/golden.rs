//! Golden-fixture tests for the pure decode/resolution engine over the maps in
//! `data/`: a basic v3 map, an index map (`sections`), an inline `sourcesContent`
//! map, and a webpack-style map. They assert the decoded token table, the
//! nearest-preceding `lookup` semantics, a no-match NULL, and inline source
//! content — the same behavior the Arrow adapters surface to SQL.

use sourcemap_worker::smap::Smap;

fn load(name: &str) -> Smap {
    let path = format!("{}/../../data/{name}", env!("CARGO_MANIFEST_DIR"));
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    Smap::parse(&bytes).unwrap_or_else(|e| panic!("parse {name}: {e}"))
}

#[test]
fn basic_v3_tokens_and_meta() {
    let m = load("basic.js.map");
    let meta = m.meta();
    assert_eq!(meta.version, 3);
    assert_eq!(meta.file.as_deref(), Some("app.min.js"));
    assert_eq!(meta.n_sources, 1);
    assert_eq!(meta.n_names, 2);
    assert_eq!(meta.n_mappings, 2);

    let toks = m.tokens();
    assert_eq!(toks.len(), 2);
    // sourceRoot is "" → source path is unprefixed.
    assert!(toks[0].source.as_deref().unwrap().ends_with("src/index.js"));
    // The first mapping starts at generated column 2 (so positions 0..2 have no
    // match — see the nearest-preceding test below).
    assert_eq!(toks[0].gen_col, 2);
    assert_eq!(toks[1].gen_col, 3);
    // The second token carries a symbol name.
    assert_eq!(toks[1].name.as_deref(), Some("greet"));
}

#[test]
fn basic_lookup_is_nearest_preceding_with_no_match_null() {
    let m = load("basic.js.map");
    // Exact hit on the first token (which starts at generated column 2).
    let r0 = m.lookup(0, 2).expect("token at (0,2)");
    assert!(r0.source.as_deref().unwrap().ends_with("src/index.js"));
    assert_eq!(r0.src_col, Some(0));

    // A column past the last token on the line resolves to the NEAREST PRECEDING
    // token (the one at gen_col 3), not an exact match.
    let r = m.lookup(0, 9).expect("nearest-preceding token");
    assert_eq!(r.src_col, Some(1));
    assert_eq!(r.name.as_deref(), Some("greet"));

    // A position BEFORE the first mapping (col 0, while the first token is at
    // col 2) has no match → NULL. (Nearest-preceding returns the last token for
    // positions *after* all mappings, so the genuine no-match is a position that
    // precedes everything.)
    assert!(
        m.lookup(0, 0).is_none(),
        "no-match must be None (→ SQL NULL)"
    );
}

#[test]
fn inline_sources_content() {
    let m = load("inline.js.map");
    let srcs = m.sources();
    assert_eq!(srcs.len(), 1);
    assert!(srcs[0].source.ends_with("inline.js"));
    assert!(srcs[0].has_content, "sourcesContent is embedded");
    assert!(m.source_content(0).unwrap().contains("answer = 42"));
    assert!(m.source_content(7).is_none(), "no such source → None");
}

#[test]
fn index_map_sections_flatten_and_resolve() {
    let m = load("index.js.map");
    let meta = m.meta();
    // Two sections, each one source / one mapping, flatten into a combined map.
    assert_eq!(meta.n_sources, 2);
    assert_eq!(meta.n_mappings, 2);

    // Section 0 covers generated line 0 (a.js); section 1 covers line 1 (b.js).
    assert!(m
        .lookup(0, 0)
        .unwrap()
        .source
        .as_deref()
        .unwrap()
        .ends_with("a.js"));
    assert!(m
        .lookup(1, 0)
        .unwrap()
        .source
        .as_deref()
        .unwrap()
        .ends_with("b.js"));
}

#[test]
fn webpack_style_map() {
    let m = load("webpack.js.map");
    assert_eq!(m.meta().n_sources, 2);
    let toks = m.tokens();
    assert_eq!(toks[0].source.as_deref(), Some("webpack:///./src/index.js"));
    // The second generated line maps into the util module.
    let r = m.lookup(1, 0).expect("token on line 1");
    assert_eq!(r.source.as_deref(), Some("webpack:///./src/util.js"));
}
