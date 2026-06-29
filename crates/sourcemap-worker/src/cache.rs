//! A tiny last-value decode cache.
//!
//! The headline workload — `lookup` / `lookup_all` over a column of error frames
//! — almost always references the SAME deployed source map on every row. Parsing
//! that map once per row would be wasteful, so we memoize the most recently
//! decoded map keyed by a hash of its bytes. A failed decode is cached too (as
//! `None`) so a column of identical bad maps doesn't re-attempt per row.
//!
//! The key is a 64-bit hash of the map bytes; the per-row cost is therefore the
//! hash (O(map size)) rather than a full re-parse (much more expensive). The
//! negligible chance of a hash collision could in principle reuse the wrong map;
//! that is an accepted trade for keeping the cache cheap and is irrelevant in
//! practice for this bulk-resolution use case.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::smap::Smap;

#[derive(Default)]
pub struct MapCache {
    last: Option<(u64, Option<Smap>)>,
}

impl MapCache {
    /// Return the decoded map for `bytes`, parsing (and caching) it only when it
    /// differs from the last call. `None` when the bytes fail to decode.
    pub fn get(&mut self, bytes: &[u8]) -> Option<&Smap> {
        let mut h = DefaultHasher::new();
        bytes.hash(&mut h);
        let key = h.finish();
        let hit = matches!(&self.last, Some((k, _)) if *k == key);
        if !hit {
            self.last = Some((key, Smap::parse(bytes).ok()));
        }
        self.last.as_ref().and_then(|(_, m)| m.as_ref())
    }
}
