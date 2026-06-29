//! Table functions exposed by the sourcemap worker, registered under
//! `sourcemap.main`: `tokens`, `sources`, and `meta`. Each takes the single
//! overloaded `map` argument (path / text / bytes) as a bind-time constant.

mod map_meta;
mod sources;
mod tokens;

use vgi::arguments::Arguments;
use vgi::Worker;
use vgi_rpc::{Result, RpcError};

use crate::maparg::map_bytes_const;
use crate::smap::Smap;

/// Register every table function on the worker.
pub fn register(worker: &mut Worker) {
    worker.register_table(tokens::Tokens);
    worker.register_table(sources::Sources);
    worker.register_table(map_meta::Meta);
}

/// Decode the constant `map` argument (positional 0) for a table function. A
/// missing/unreadable path or malformed map is a query error here (the whole
/// call is about that one map), unlike the per-row NULL policy of `lookup`.
pub(crate) fn load_const_map(args: &Arguments) -> Result<Smap> {
    let bytes = map_bytes_const(args, 0)?;
    Smap::parse(&bytes).map_err(|e| RpcError::value_error(format!("invalid source map: {e}")))
}
