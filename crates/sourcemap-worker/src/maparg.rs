//! Resolving the overloaded `map` argument to raw `.map` bytes.
//!
//! Every function takes its source map the same way — a single overloaded `map`
//! argument that is one of:
//!
//! - a **path** (`VARCHAR`) to a `.map` file on the worker's filesystem,
//! - the raw map **text** (`VARCHAR` JSON, e.g. from `read_text(...)` or a
//!   `data:` URI's payload), or
//! - the map **bytes** (`BLOB`).
//!
//! A `VARCHAR` is treated as inline JSON text when it begins — after optional
//! leading whitespace and the `)]}'` XSSI guard some toolchains prepend — with
//! `{`; otherwise it is read as a filesystem path. `BLOB` is always used
//! verbatim. This mirrors how the headline queries pass either a literal path
//! (`tokens('/dist/app.min.js.map')`) or fetched text
//! (`lookup(read_text(e.map_path), …)`).

use arrow_array::cast::AsArray;
use arrow_array::{Array, ArrayRef};
use arrow_schema::DataType;
use vgi::arguments::Arguments;
use vgi_rpc::{Result, RpcError};

/// Does this VARCHAR look like inline source-map JSON (rather than a path)?
fn looks_like_json(s: &str) -> bool {
    let t = s.trim_start();
    t.starts_with('{') || t.starts_with(")]}")
}

/// Resolve a VARCHAR map cell to bytes: inline JSON text → its own bytes;
/// otherwise read the file at that path.
fn bytes_from_str(s: &str) -> Result<Vec<u8>> {
    if looks_like_json(s) {
        Ok(s.as_bytes().to_vec())
    } else {
        std::fs::read(s).map_err(|e| RpcError::value_error(format!("source map path '{s}': {e}")))
    }
}

/// Resolve the overloaded `map` cell at `row` of `col` to raw map bytes, or
/// `None` if the cell is NULL. Errors only on an unreadable path or an
/// unsupported column type — the caller decides whether that is a per-row NULL
/// (scalars / `lookup_all`) or a query error (whole-map table functions).
pub fn map_bytes_cell(col: &ArrayRef, row: usize) -> Result<Option<Vec<u8>>> {
    if col.is_null(row) {
        return Ok(None);
    }
    Ok(Some(match col.data_type() {
        DataType::Utf8 => bytes_from_str(col.as_string::<i32>().value(row))?,
        DataType::LargeUtf8 => bytes_from_str(col.as_string::<i64>().value(row))?,
        DataType::Utf8View => bytes_from_str(col.as_string_view().value(row))?,
        DataType::Binary => col.as_binary::<i32>().value(row).to_vec(),
        DataType::LargeBinary => col.as_binary::<i64>().value(row).to_vec(),
        DataType::BinaryView => col.as_binary_view().value(row).to_vec(),
        other => {
            return Err(RpcError::value_error(format!(
                "the `map` argument must be a VARCHAR (path or JSON text) or BLOB, got {other:?}"
            )))
        }
    }))
}

/// Resolve a positional **const** `map` argument (table functions) to bytes.
pub fn map_bytes_const(args: &Arguments, pos: usize) -> Result<Vec<u8>> {
    let arr = args
        .arg(pos)
        .ok_or_else(|| RpcError::value_error("the `map` argument is required"))?;
    map_bytes_cell(arr, 0)?
        .ok_or_else(|| RpcError::value_error("the `map` argument must not be NULL"))
}

/// Resolve a **named** `map` argument (the `map :=` of `lookup_all`) to bytes.
pub fn map_bytes_named(args: &Arguments, name: &str) -> Result<Vec<u8>> {
    let arr = args
        .named(name)
        .ok_or_else(|| RpcError::value_error(format!("the `{name} :=` argument is required")))?;
    map_bytes_cell(arr, 0)?
        .ok_or_else(|| RpcError::value_error(format!("the `{name} :=` argument must not be NULL")))
}
