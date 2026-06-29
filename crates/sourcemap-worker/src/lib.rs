//! Library surface of the `sourcemap` VGI worker.
//!
//! The binary (`main.rs`) is the actual worker; this `lib` target exposes the
//! pure Source Map v3 decode/resolution engine so integration tests under
//! `tests/` (and downstream consumers) can exercise it directly, without Arrow
//! or RPC. See [`smap`] for the engine.

pub mod smap;
