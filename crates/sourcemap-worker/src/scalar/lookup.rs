//! `lookup(map, gen_line, gen_col) -> STRUCT(source, src_line, src_col, name)`.
//!
//! Resolve a single minified `(gen_line, gen_col)` position back to its original
//! source location using **nearest-preceding** semantics (the standard
//! source-map lookup), so an exact mapping for the position isn't required — that
//! is how real stack frames resolve. `map` is the overloaded path/text/bytes
//! argument (see [`crate::maparg`]); it is read per row, so it may be a literal,
//! a constant, or a column like `read_text(e.map_path)`.
//!
//! Per-row safety: a NULL / malformed / empty map, a NULL position, or a
//! position that precedes the first mapping all yield a **NULL struct** for that
//! row — never a query error. Positions are 0-indexed (Source Map v3).

use std::sync::Arc;

use arrow_array::builder::{Int64Builder, StringBuilder};
use arrow_array::{ArrayRef, RecordBatch, StructArray};
use arrow_buffer::NullBuffer;
use arrow_schema::DataType;
use vgi::{
    ArgSpec, BindParams, BindResponse, FunctionExample, FunctionMetadata, ProcessParams,
    ScalarFunction,
};
use vgi_rpc::{Result, RpcError};

use crate::arrow_io::{pos_u32, resolved_struct_fields};
use crate::cache::MapCache;
use crate::maparg::map_bytes_cell;
use crate::smap::Resolved;

pub struct Lookup;

impl Lookup {
    /// Resolve one row, or `None` (→ NULL struct) on any missing/unresolvable input.
    fn resolve_row(
        cache: &mut MapCache,
        map: &ArrayRef,
        line: &ArrayRef,
        col: &ArrayRef,
        i: usize,
    ) -> Resolved_ {
        // Bad path / unsupported type / NULL → unresolvable (NULL), not an error.
        let bytes = match map_bytes_cell(map, i) {
            Ok(Some(b)) => b,
            _ => return Resolved_::None,
        };
        let (line, col) = match (pos_u32(line, i), pos_u32(col, i)) {
            (Ok(Some(l)), Ok(Some(c))) => (l, c),
            _ => return Resolved_::None,
        };
        match cache.get(&bytes) {
            Some(smap) => match smap.lookup(line, col) {
                Some(r) => Resolved_::Some(r),
                None => Resolved_::None,
            },
            None => Resolved_::None,
        }
    }
}

/// Local result type (avoids leaking a per-row error out of the closure).
enum Resolved_ {
    Some(Resolved),
    None,
}

impl ScalarFunction for Lookup {
    fn name(&self) -> &str {
        "lookup"
    }

    fn metadata(&self) -> FunctionMetadata {
        FunctionMetadata {
            description: "Resolve a minified (gen_line, gen_col) position to its original \
                          STRUCT(source, src_line, src_col, name) using nearest-preceding \
                          source-map semantics; NULL if the map is missing/malformed or the \
                          position precedes the first mapping"
                .into(),
            return_type: Some(DataType::Struct(resolved_struct_fields())),
            examples: vec![FunctionExample {
                sql: "SELECT sourcemap.main.lookup('{\"version\":3,\"sources\":[\"index.js\"],\
                      \"names\":[],\"mappings\":\"AAAA\"}', 0, 0);"
                    .into(),
                description: "Resolve a generated line/column back to its original location."
                    .into(),
                expected_output: None,
            }],
            tags: crate::meta::object_tags(
                "Resolve Minified Position",
                "Resolve one minified (generated) line/column position back to its original \
                 source location using nearest-preceding source-map semantics. Returns a \
                 STRUCT(source VARCHAR, src_line BIGINT, src_col BIGINT, name VARCHAR). The `map` \
                 argument is a path to a .map file, the raw map JSON text, or map BLOB bytes. All \
                 line/column numbers are 0-indexed. Returns NULL when the map is NULL, malformed, \
                 or empty, when a position is NULL, or when the position precedes the first \
                 mapping. Use it to de-minify a single error frame.",
                "Resolve a minified `(gen_line, gen_col)` to its original \
                 `STRUCT(source, src_line, src_col, name)`, e.g. \
                 `lookup(read_text(map_path), line, col)`. Nearest-preceding; 0-indexed; NULL on \
                 no match.",
                "lookup, resolve, de-minify, deminify, source map, original position, stack frame, \
                 symbolicate, line column, nearest preceding, RUM, error tracking",
                "Resolution",
            ),
            ..Default::default()
        }
    }

    fn argument_specs(&self) -> Vec<ArgSpec> {
        vec![
            ArgSpec::any_column(
                "map",
                0,
                "The source map: a path to a .map file, the raw map JSON text (e.g. from \
                 read_text(...)), or the map bytes. Read per row, so it may vary per row \
                 (e.g. read_text(e.map_path)).",
            ),
            ArgSpec::column_typed(
                "gen_line",
                1,
                DataType::Int64,
                "The 0-indexed generated (minified) line to resolve.",
            ),
            ArgSpec::column_typed(
                "gen_col",
                2,
                DataType::Int64,
                "The 0-indexed generated (minified) column to resolve.",
            ),
        ]
    }

    fn on_bind(&self, _params: &BindParams) -> Result<BindResponse> {
        Ok(BindResponse::result(DataType::Struct(
            resolved_struct_fields(),
        )))
    }

    fn process(&self, params: &ProcessParams, batch: &RecordBatch) -> Result<RecordBatch> {
        let map = batch.column(0);
        let line = batch.column(1);
        let col = batch.column(2);
        let rows = batch.num_rows();

        let mut source = StringBuilder::new();
        let mut src_line = Int64Builder::new();
        let mut src_col = Int64Builder::new();
        let mut name = StringBuilder::new();
        let mut valid: Vec<bool> = Vec::with_capacity(rows);
        let mut cache = MapCache::default();

        for i in 0..rows {
            match Self::resolve_row(&mut cache, map, line, col, i) {
                Resolved_::Some(r) => {
                    append_opt_str(&mut source, r.source.as_deref());
                    append_opt_i64(&mut src_line, r.src_line);
                    append_opt_i64(&mut src_col, r.src_col);
                    append_opt_str(&mut name, r.name.as_deref());
                    valid.push(true);
                }
                Resolved_::None => {
                    source.append_null();
                    src_line.append_null();
                    src_col.append_null();
                    name.append_null();
                    valid.push(false);
                }
            }
        }

        let arrays: Vec<ArrayRef> = vec![
            Arc::new(source.finish()),
            Arc::new(src_line.finish()),
            Arc::new(src_col.finish()),
            Arc::new(name.finish()),
        ];
        let out: ArrayRef = Arc::new(StructArray::new(
            resolved_struct_fields(),
            arrays,
            Some(NullBuffer::from(valid)),
        ));
        RecordBatch::try_new(params.output_schema.clone(), vec![out])
            .map_err(|e| RpcError::runtime_error(e.to_string()))
    }
}

fn append_opt_str(b: &mut StringBuilder, v: Option<&str>) {
    match v {
        Some(s) => b.append_value(s),
        None => b.append_null(),
    }
}

fn append_opt_i64(b: &mut Int64Builder, v: Option<u32>) {
    match v {
        Some(n) => b.append_value(n as i64),
        None => b.append_null(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arrow_io::test_support::run_scalar_on;
    use arrow_array::cast::AsArray;
    use arrow_array::types::Int64Type;
    use arrow_array::{Array, Int64Array, StringArray};
    use arrow_schema::Field;

    const BASIC: &str = r#"{"version":3,"file":"app.min.js","sources":["index.js"],"names":["greet","name"],"mappings":"AAAA,MAAMA,MAAQC,GAAS"}"#;

    fn run(maps: Vec<Option<&str>>, lines: Vec<Option<i64>>, cols: Vec<Option<i64>>) -> ArrayRef {
        let m: ArrayRef = Arc::new(StringArray::from(maps));
        let l: ArrayRef = Arc::new(Int64Array::from(lines));
        let c: ArrayRef = Arc::new(Int64Array::from(cols));
        run_scalar_on(
            &Lookup,
            vec![m, l, c],
            vec![
                Field::new("map", DataType::Utf8, true),
                Field::new("gen_line", DataType::Int64, true),
                Field::new("gen_col", DataType::Int64, true),
            ],
        )
        .unwrap()
    }

    #[test]
    fn binds_resolved_struct() {
        let out = run(vec![Some(BASIC)], vec![Some(0)], vec![Some(0)]);
        assert_eq!(out.data_type(), &DataType::Struct(resolved_struct_fields()));
    }

    #[test]
    fn resolves_inline_text_map() {
        let out = run(vec![Some(BASIC)], vec![Some(0)], vec![Some(0)]);
        let s = out.as_struct();
        assert!(!out.is_null(0));
        assert_eq!(s.column(0).as_string::<i32>().value(0), "index.js");
        // src_line/col are 0-indexed BIGINT.
        assert_eq!(s.column(1).as_primitive::<Int64Type>().value(0), 0);
    }

    #[test]
    fn malformed_and_null_yield_null_struct() {
        let out = run(
            vec![Some("not a map"), None, Some(BASIC)],
            vec![Some(0), Some(0), None],
            vec![Some(0), Some(0), Some(0)],
        );
        assert!(out.is_null(0), "malformed map → NULL");
        assert!(out.is_null(1), "NULL map → NULL");
        assert!(out.is_null(2), "NULL position → NULL");
    }
}
