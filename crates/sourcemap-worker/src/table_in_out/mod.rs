//! Table-in-out functions exposed by the sourcemap worker.
//!
//! `lookup_all` is the practical bulk entry point: stream a column of minified
//! error frames `(id, line, col)` in, get resolved original locations out, with
//! the `id` carried through so the result joins straight back to the originating
//! error row.

mod lookup_all;

use vgi::Worker;

/// Register every table-in-out function on the worker.
pub fn register(worker: &mut Worker) {
    worker.register_table_in_out(lookup_all::LookupAll);
}
