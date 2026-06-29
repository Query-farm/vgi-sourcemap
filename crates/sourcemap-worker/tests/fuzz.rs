//! Property-based fuzzing: the decoder must NEVER panic, on any input.
//!
//! `Smap::parse` is the single entry point for untrusted bytes (a column of map
//! paths / text / blobs straight from user data). It must always return either a
//! decoded map or a `DecodeError` — never panic, never abort — so that the
//! per-row NULL safety policy holds. We throw arbitrary bytes, arbitrary
//! truncations of a valid map, and structurally-plausible-but-corrupt JSON at it
//! and assert only that it returns.

use proptest::prelude::*;
use sourcemap_worker::smap::Smap;

const VALID: &str = r#"{"version":3,"file":"o.js","sourceRoot":"/r","sources":["a.js","b.js"],"sourcesContent":["x","y"],"names":["n"],"mappings":"AAAA,CAACA;ACDD"}"#;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4000))]

    /// Arbitrary bytes never panic the decoder.
    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
        let _ = Smap::parse(&bytes);
    }

    /// Every truncation/prefix of a valid map decodes-or-errors, never panics.
    #[test]
    fn truncated_valid_map_never_panics(n in 0usize..=VALID.len()) {
        let _ = Smap::parse(&VALID.as_bytes()[..n]);
    }

    /// A valid map with one byte flipped never panics (corrupt VLQ / JSON).
    #[test]
    fn single_byte_corruption_never_panics(
        idx in 0usize..VALID.len(),
        b in any::<u8>(),
    ) {
        let mut bytes = VALID.as_bytes().to_vec();
        bytes[idx] = b;
        let _ = Smap::parse(&bytes);
    }

    /// Random ASCII (often JSON-ish) never panics.
    #[test]
    fn random_ascii_never_panics(s in "[ -~]{0,256}") {
        let _ = Smap::parse(s.as_bytes());
    }
}
