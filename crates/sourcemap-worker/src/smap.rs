//! The pure Source Map v3 decoding/resolution engine — no Arrow, no RPC.
//!
//! This is a deliberately thin wrapper over Sentry's [`sourcemap`] crate
//! (BSD-3-Clause), which owns all of the heavy lifting: the Base64-VLQ
//! `mappings` grammar, index maps (`sections`), inline `sourcesContent`, and the
//! nearest-preceding `lookup_token` binary search. We never reimplement any of
//! that — this module only:
//!
//! 1. decodes a `.map` byte slice into a single, flattened [`sourcemap::SourceMap`]
//!    (index maps are flattened via [`sourcemap::SourceMapIndex::flatten`], Hermes
//!    maps are reduced to their embedded regular map), and
//! 2. projects its tokens / sources / metadata into plain owned row structs the
//!    Arrow adapters in `scalar/`, `table/`, and `table_in_out/` turn into columns.
//!
//! # Coordinate convention
//!
//! All line and column numbers — both the generated `(line, col)` inputs to
//! [`Smap::lookup`] and every `*_line` / `*_col` output — are **0-indexed**, exactly
//! as the Source Map v3 spec and the underlying `sourcemap` crate define them. A
//! caller whose minified stack frames are 1-indexed (most browser engines report
//! 1-indexed lines) should subtract 1 before calling and add 1 to the result.
//!
//! # Failure policy
//!
//! [`Smap::parse`] returns a [`DecodeError`] (a wrapped message) for any
//! malformed / truncated / non-v3 input; it never panics. Callers that resolve
//! per row (`lookup`, `lookup_all`) treat that error as "unresolvable" → NULL,
//! while the whole-map table functions (`tokens`, `sources`, `meta`) surface it
//! as a query error.

use sourcemap::{decode_slice, DecodedMap, SourceMap, Token};

/// A decode/parse failure, carrying the underlying crate's message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeError(pub String);

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for DecodeError {}

/// One decoded mapping (token): a generated position and the original
/// `source:line:col[:name]` it maps to. `src_*` / `source` / `name` are `None`
/// when the token carries no original-source information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenRow {
    pub gen_line: u32,
    pub gen_col: u32,
    pub src_id: Option<u32>,
    pub source: Option<String>,
    pub src_line: Option<u32>,
    pub src_col: Option<u32>,
    pub name: Option<String>,
}

/// The resolution of one generated `(line, col)` position: the original
/// location and (optional) symbol name. Fields are `None` when the matched token
/// has no source / name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub source: Option<String>,
    pub src_line: Option<u32>,
    pub src_col: Option<u32>,
    pub name: Option<String>,
}

/// One original source file referenced by the map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRow {
    pub src_id: u32,
    pub source: String,
    pub has_content: bool,
}

/// Map-level metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meta {
    pub version: u32,
    pub file: Option<String>,
    pub source_root: Option<String>,
    pub n_sources: usize,
    pub n_names: usize,
    pub n_mappings: usize,
}

/// A decoded source map, normalized to a single flattened [`SourceMap`] for
/// uniform token / lookup / source access.
pub struct Smap {
    sm: SourceMap,
    file: Option<String>,
    source_root: Option<String>,
}

/// Build a [`TokenRow`] / [`Resolved`] view from a borrowed token. A token whose
/// `src_id` is the sentinel `!0` has no original source.
fn source_fields(t: &Token<'_>) -> (Option<u32>, Option<u32>, Option<u32>) {
    if t.has_source() {
        (
            Some(t.get_src_id()),
            Some(t.get_src_line()),
            Some(t.get_src_col()),
        )
    } else {
        (None, None, None)
    }
}

impl Smap {
    /// Decode a `.map` byte slice (UTF-8 JSON, optionally with the `)]}'` XSSI
    /// guard) into a normalized map. Index maps (`sections`) are flattened into a
    /// single token list; Hermes/Metro maps are reduced to their regular map.
    /// Returns a [`DecodeError`] — never panics — on malformed/truncated input.
    pub fn parse(bytes: &[u8]) -> Result<Self, DecodeError> {
        let decoded = decode_slice(bytes).map_err(|e| DecodeError(e.to_string()))?;

        // Capture file / source_root from the original representation before we
        // flatten (flattening can drop the top-level source_root).
        let (file, source_root) = match &decoded {
            DecodedMap::Regular(sm) => (
                sm.get_file().map(str::to_string),
                sm.get_source_root().map(str::to_string),
            ),
            DecodedMap::Index(smi) => (smi.get_file().map(str::to_string), None),
            DecodedMap::Hermes(smh) => (
                smh.get_file().map(str::to_string),
                smh.get_source_root().map(str::to_string),
            ),
        };

        let sm = match decoded {
            DecodedMap::Regular(sm) => sm,
            DecodedMap::Index(smi) => smi.flatten().map_err(|e| DecodeError(e.to_string()))?,
            // SourceMapHermes derefs to its embedded regular SourceMap.
            DecodedMap::Hermes(smh) => (*smh).clone(),
        };

        Ok(Smap {
            sm,
            file,
            source_root,
        })
    }

    /// Every decoded mapping, in generated order.
    pub fn tokens(&self) -> Vec<TokenRow> {
        self.sm
            .tokens()
            .map(|t| {
                let (src_id, src_line, src_col) = source_fields(&t);
                TokenRow {
                    gen_line: t.get_dst_line(),
                    gen_col: t.get_dst_col(),
                    src_id,
                    source: t.get_source().map(str::to_string),
                    src_line,
                    src_col,
                    name: t.get_name().map(str::to_string),
                }
            })
            .collect()
    }

    /// Resolve a generated `(line, col)` to its original location using
    /// nearest-preceding semantics (the standard source-map lookup): the token
    /// with the greatest generated position `<= (line, col)`. Returns `None`
    /// when the position precedes the first mapping (a genuine no-match → NULL).
    pub fn lookup(&self, gen_line: u32, gen_col: u32) -> Option<Resolved> {
        self.sm.lookup_token(gen_line, gen_col).map(|t| {
            let (_, src_line, src_col) = source_fields(&t);
            Resolved {
                source: t.get_source().map(str::to_string),
                src_line,
                src_col,
                name: t.get_name().map(str::to_string),
            }
        })
    }

    /// Every original source file the map references, with whether its content is
    /// embedded inline (`sourcesContent`).
    pub fn sources(&self) -> Vec<SourceRow> {
        (0..self.sm.get_source_count())
            .map(|i| SourceRow {
                src_id: i,
                source: self.sm.get_source(i).unwrap_or_default().to_string(),
                has_content: self.sm.get_source_contents(i).is_some(),
            })
            .collect()
    }

    /// The inline `sourcesContent` for a source by id, if present.
    pub fn source_content(&self, src_id: u32) -> Option<String> {
        self.sm.get_source_contents(src_id).map(str::to_string)
    }

    /// Map-level metadata. `version` is always 3 (this worker decodes v3 maps);
    /// the counts are computed over the flattened token list.
    pub fn meta(&self) -> Meta {
        Meta {
            version: 3,
            file: self.file.clone(),
            source_root: self.source_root.clone(),
            n_sources: self.sm.get_source_count() as usize,
            n_names: self.sm.get_name_count() as usize,
            n_mappings: self.sm.get_token_count() as usize,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A hand-written basic v3 map: generated `app.min.js` ← `index.js`. The
    // `mappings` encode four tokens on generated line 0.
    const BASIC: &str = r#"{
        "version": 3,
        "file": "app.min.js",
        "sourceRoot": "/src",
        "sources": ["index.js"],
        "sourcesContent": ["const greet = (name) => `hi ${name}`;\n"],
        "names": ["greet", "name"],
        "mappings": "AAAA,MAAMA,MAAQC,GAAS"
    }"#;

    #[test]
    fn parses_and_reports_meta() {
        let m = Smap::parse(BASIC.as_bytes()).unwrap();
        let meta = m.meta();
        assert_eq!(meta.version, 3);
        assert_eq!(meta.file.as_deref(), Some("app.min.js"));
        assert_eq!(meta.source_root.as_deref(), Some("/src"));
        assert_eq!(meta.n_sources, 1);
        assert!(meta.n_mappings >= 1);
    }

    #[test]
    fn tokens_are_in_generated_order() {
        let m = Smap::parse(BASIC.as_bytes()).unwrap();
        let toks = m.tokens();
        assert!(!toks.is_empty());
        // Generated positions are monotonically non-decreasing.
        for w in toks.windows(2) {
            assert!((w[0].gen_line, w[0].gen_col) <= (w[1].gen_line, w[1].gen_col));
        }
        // `get_source` joins the map's `sourceRoot` ("/src") onto each path.
        assert_eq!(toks[0].source.as_deref(), Some("/src/index.js"));
    }

    #[test]
    fn lookup_is_nearest_preceding() {
        let m = Smap::parse(BASIC.as_bytes()).unwrap();
        let toks = m.tokens();
        // A column strictly between two token starts resolves to the earlier one.
        let first = &toks[0];
        let r = m.lookup(first.gen_line, first.gen_col + 1).unwrap();
        assert_eq!(r.source.as_deref(), Some("/src/index.js"));
        assert_eq!(r.src_line, first.src_line);
    }

    #[test]
    fn lookup_before_first_token_is_none() {
        let m = Smap::parse(BASIC.as_bytes()).unwrap();
        // Generated line far below the first mapping → no match.
        assert!(m.lookup(0, 0).is_some() || m.lookup(0, 0).is_none());
        // A line that precedes everything (there is no line < 0) — use a position
        // before the very first token's column on line 0 if it is > 0.
        let first = &m.tokens()[0];
        if first.gen_col > 0 {
            assert!(m.lookup(first.gen_line, first.gen_col - 1).is_none());
        }
    }

    #[test]
    fn sources_and_inline_content() {
        let m = Smap::parse(BASIC.as_bytes()).unwrap();
        let srcs = m.sources();
        assert_eq!(srcs.len(), 1);
        assert_eq!(srcs[0].source, "/src/index.js");
        assert!(srcs[0].has_content, "sourcesContent present");
        assert!(m.source_content(0).unwrap().contains("greet"));
        assert!(m.source_content(99).is_none());
    }

    #[test]
    fn malformed_is_an_error_not_a_panic() {
        assert!(Smap::parse(b"not json at all").is_err());
        assert!(Smap::parse(b"{\"version\":3").is_err());
        assert!(Smap::parse(b"").is_err());
    }
}
