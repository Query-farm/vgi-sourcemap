//! Scalar functions exposed by the sourcemap worker, registered under
//! `sourcemap.main`.

mod lookup;
mod source_content;

use vgi::Worker;

/// Register every scalar function on the worker.
pub fn register(worker: &mut Worker) {
    worker.register_scalar(lookup::Lookup);
    worker.register_scalar(source_content::SourceContent);
}
