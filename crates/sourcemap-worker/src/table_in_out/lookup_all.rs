//! `lookup_all((SELECT id, line, col FROM frames), map := …, id := 'id'
//!   [, line := 'line', col := 'col'])`
//!
//! Resolve a whole column of minified `(line, col)` frames against ONE source
//! map, streaming `(id, source, src_line, src_col, name)` rows out. The `id`
//! column is **passed through** unchanged (same name and type as the input), so
//! the result joins straight back to the originating error row — the
//! `vgi-tika`/sklearn table-in-out convention.
//!
//! - `map :=` is the single map (path / JSON text / BLOB) applied to every row.
//! - `id :=` / `line :=` / `col :=` name the input columns (defaults `id`,
//!   `line`, `col`). The id column may be any type; line/col are read as integers.
//! - All positions are 0-indexed; resolution is nearest-preceding.
//! - Per-row safety: a malformed/empty map, a NULL position, or a no-match all
//!   yield NULL resolution columns for that row (the id still passes through) —
//!   never a query failure.

use std::sync::Arc;

use arrow_array::builder::{Int64Builder, StringBuilder};
use arrow_array::{ArrayRef, RecordBatch};
use arrow_schema::{DataType, Field, Schema};
use vgi::arguments::Arguments;
use vgi::function::{ArgSpec, BindParams, BindResponse, FunctionMetadata, ProcessParams};
use vgi::table_in_out::TableInOutFunction;
use vgi_rpc::{Result, RpcError};

use crate::arrow_io::pos_u32;
use crate::maparg::map_bytes_named;
use crate::smap::Smap;

pub struct LookupAll;

/// Resolve a named column-name argument, falling back to `default`.
fn name_of(args: &Arguments, key: &str, default: &str) -> String {
    args.named_str(key).unwrap_or_else(|| default.to_string())
}

/// The four appended resolution columns (after the passthrough id).
fn resolution_fields() -> Vec<Field> {
    vec![
        Field::new("source", DataType::Utf8, true),
        Field::new("src_line", DataType::Int64, true),
        Field::new("src_col", DataType::Int64, true),
        Field::new("name", DataType::Utf8, true),
    ]
}

impl TableInOutFunction for LookupAll {
    fn name(&self) -> &str {
        "lookup_all"
    }

    fn metadata(&self) -> FunctionMetadata {
        let mut tags = crate::meta::object_tags(
            "Bulk Resolve Minified Frames",
            "Resolve a whole column of minified (line, col) error frames against ONE source map, \
             returning the original (source, src_line, src_col, name) for each — with the input \
             id column passed through so the result joins back to the originating row. The input \
             relation is supplied as a subquery (e.g. `(SELECT id, line, col FROM frames)`); the \
             single map is the named `map :=` argument (a .map path, raw JSON text, or BLOB). \
             Name the id/line/col columns with `id :=` / `line :=` / `col :=` (defaults 'id', \
             'line', 'col'). Positions are 0-indexed; resolution is nearest-preceding. A \
             malformed map, NULL position, or no-match yields NULL resolution columns for that \
             row (the id still passes through). This is the bulk de-minify entry point for RUM / \
             error-tracking frames.",
            "Bulk-resolve a column of minified `(line, col)` frames against one map, with `id` \
             passthrough: `lookup_all((SELECT id, line, col FROM frames), map := …, id := 'id')`. \
             Emits `(id, source, src_line, src_col, name)`.",
            "lookup_all, bulk resolve, de-minify, deminify, error frames, RUM, error tracking, \
             stack frames, id passthrough, symbolicate, table function, resolve column",
        );
        tags.push((
            "vgi.result_columns_md".into(),
            "The output is the passthrough **id** column (same name and type as the input) \
             followed by:\n\n\
             | column | type | description |\n\
             |---|---|---|\n\
             | `source` | VARCHAR | original source file (NULL if unresolved). |\n\
             | `src_line` | BIGINT | 0-indexed original line (NULL if unresolved). |\n\
             | `src_col` | BIGINT | 0-indexed original column (NULL if unresolved). |\n\
             | `name` | VARCHAR | original symbol name (NULL if none). |"
                .into(),
        ));
        tags.push((
            "vgi.executable_examples".into(),
            r#"[
  {
    "description": "Resolve a column of frames against one inline map, joining back by id.",
    "sql": "SELECT r.id, r.source, r.src_line FROM sourcemap.main.lookup_all((SELECT * FROM (VALUES (1, 0, 0)) AS f(id, line, col)), map := '{\"version\":3,\"file\":\"o.js\",\"sources\":[\"index.js\"],\"names\":[],\"mappings\":\"AAAA\"}', id := 'id') AS r"
  }
]"#
            .into(),
        ));
        FunctionMetadata {
            description:
                "Bulk-resolve a column of minified frames against one map (id passthrough)".into(),
            tags,
            ..Default::default()
        }
    }

    fn argument_specs(&self) -> Vec<ArgSpec> {
        vec![
            ArgSpec::column(
                "frames",
                0,
                "table",
                "The input relation of minified frames, supplied as a subquery, e.g. \
                 `(SELECT id, line, col FROM frames)`. Must contain the id/line/col columns named \
                 by the `id`/`line`/`col` arguments.",
            ),
            ArgSpec::const_arg(
                "map",
                -1,
                "any",
                "The single source map applied to every input row: a path to a .map file, the \
                 raw map JSON text, or the map bytes.",
            ),
            ArgSpec::const_arg(
                "id",
                -1,
                "varchar",
                "Name of the input column to pass through to the output unchanged (so results \
                 join back to the source row). Default 'id'.",
            ),
            ArgSpec::const_arg(
                "line",
                -1,
                "varchar",
                "Name of the input column holding the 0-indexed generated line. Default 'line'.",
            ),
            ArgSpec::const_arg(
                "col",
                -1,
                "varchar",
                "Name of the input column holding the 0-indexed generated column. Default 'col'.",
            ),
        ]
    }

    fn on_bind(&self, params: &BindParams) -> Result<BindResponse> {
        let input = params
            .input_schema
            .clone()
            .ok_or_else(|| RpcError::value_error("lookup_all requires an input relation"))?;

        let id_name = name_of(&params.arguments, "id", "id");
        let line_name = name_of(&params.arguments, "line", "line");
        let col_name = name_of(&params.arguments, "col", "col");

        // The id column is passed through verbatim — keep its exact field.
        let id_field = input
            .column_with_name(&id_name)
            .map(|(_, f)| f.clone())
            .ok_or_else(|| {
                RpcError::value_error(format!(
                    "lookup_all: id column '{id_name}' not found in the input relation"
                ))
            })?;
        // Validate the position columns exist up front (fail loud on misconfig).
        for (label, name) in [("line", &line_name), ("col", &col_name)] {
            if input.column_with_name(name).is_none() {
                return Err(RpcError::value_error(format!(
                    "lookup_all: {label} column '{name}' not found in the input relation"
                )));
            }
        }

        let mut fields = vec![id_field];
        fields.extend(resolution_fields());
        Ok(BindResponse {
            output_schema: Arc::new(Schema::new(fields)),
            opaque_data: Vec::new(),
        })
    }

    fn process(&self, params: &ProcessParams, batch: &RecordBatch) -> Result<Vec<RecordBatch>> {
        let id_name = name_of(&params.arguments, "id", "id");
        let line_name = name_of(&params.arguments, "line", "line");
        let col_name = name_of(&params.arguments, "col", "col");

        // Resolve the input columns by name. The id passthrough is the input
        // array verbatim; line/col are read per row.
        let id_col = batch
            .column_by_name(&id_name)
            .ok_or_else(|| RpcError::runtime_error(format!("missing id column '{id_name}'")))?
            .clone();
        let line_col = batch
            .column_by_name(&line_name)
            .ok_or_else(|| RpcError::runtime_error(format!("missing line column '{line_name}'")))?;
        let col_col = batch
            .column_by_name(&col_name)
            .ok_or_else(|| RpcError::runtime_error(format!("missing col column '{col_name}'")))?;

        // Decode the single map ONCE per batch. A bad map → every row resolves to
        // NULL (the id still passes through) rather than failing the query.
        let smap: Option<Smap> = match map_bytes_named(&params.arguments, "map") {
            Ok(bytes) => Smap::parse(&bytes).ok(),
            Err(_) => None,
        };

        let rows = batch.num_rows();
        let mut source = StringBuilder::new();
        let mut src_line = Int64Builder::new();
        let mut src_col = Int64Builder::new();
        let mut name = StringBuilder::new();

        for i in 0..rows {
            let resolved = smap.as_ref().and_then(|m| {
                let l = pos_u32(line_col, i).ok().flatten()?;
                let c = pos_u32(col_col, i).ok().flatten()?;
                m.lookup(l, c)
            });
            match resolved {
                Some(r) => {
                    match r.source {
                        Some(s) => source.append_value(s),
                        None => source.append_null(),
                    }
                    match r.src_line {
                        Some(v) => src_line.append_value(v as i64),
                        None => src_line.append_null(),
                    }
                    match r.src_col {
                        Some(v) => src_col.append_value(v as i64),
                        None => src_col.append_null(),
                    }
                    match r.name {
                        Some(s) => name.append_value(s),
                        None => name.append_null(),
                    }
                }
                None => {
                    source.append_null();
                    src_line.append_null();
                    src_col.append_null();
                    name.append_null();
                }
            }
        }

        let cols: Vec<ArrayRef> = vec![
            id_col,
            Arc::new(source.finish()),
            Arc::new(src_line.finish()),
            Arc::new(src_col.finish()),
            Arc::new(name.finish()),
        ];
        let out = RecordBatch::try_new(params.output_schema.clone(), cols)
            .map_err(|e| RpcError::runtime_error(e.to_string()))?;
        Ok(vec![out])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arrow_io::test_support::process_params;
    use arrow_array::cast::AsArray;
    use arrow_array::types::Int64Type;
    use arrow_array::{Array, Int64Array, RecordBatch, StringArray};
    use arrow_schema::Schema;
    use vgi::arguments::Arguments;

    const MAP: &str =
        r#"{"version":3,"file":"o.js","sources":["index.js"],"names":[],"mappings":"AAAA"}"#;

    /// Build `Arguments` carrying the named `map` (and optional id/line/col names).
    fn args(map: &str) -> Arguments {
        let mut a = Arguments::default();
        a.named
            .insert("map".to_string(), Arc::new(StringArray::from(vec![map])));
        a
    }

    fn frames() -> (RecordBatch, std::sync::Arc<Schema>) {
        let schema = std::sync::Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("line", DataType::Int64, true),
            Field::new("col", DataType::Int64, true),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int64Array::from(vec![10, 20, 30])) as ArrayRef,
                Arc::new(Int64Array::from(vec![Some(0), Some(0), None])) as ArrayRef,
                Arc::new(Int64Array::from(vec![Some(0), Some(5), Some(0)])) as ArrayRef,
            ],
        )
        .unwrap();
        (batch, schema)
    }

    #[test]
    fn passes_id_through_and_resolves() {
        let (batch, schema) = frames();
        let arguments = args(MAP);
        let bind = BindParams {
            input_schema: Some(schema),
            arguments: arguments.clone(),
            ..Default::default()
        };
        let bound = LookupAll.on_bind(&bind).unwrap();
        // The passthrough id field keeps its name and type.
        assert_eq!(bound.output_schema.field(0).name(), "id");
        assert_eq!(bound.output_schema.field(0).data_type(), &DataType::Int64);

        let params = process_params(bound.output_schema, arguments);
        let out = LookupAll.process(&params, &batch).unwrap();
        assert_eq!(out.len(), 1);
        let b = &out[0];
        // id passthrough, unchanged values.
        let ids = b.column(0).as_primitive::<Int64Type>();
        assert_eq!((ids.value(0), ids.value(1), ids.value(2)), (10, 20, 30));
        // rows 0 and 1 resolve (nearest-preceding) to index.js; row 2 has a NULL
        // position → NULL resolution but the id still passes through.
        let src = b.column(1).as_string::<i32>();
        assert_eq!(src.value(0), "index.js");
        assert_eq!(src.value(1), "index.js");
        assert!(b.column(1).is_null(2), "NULL position → NULL source");
    }

    #[test]
    fn malformed_map_keeps_id_passthrough_all_null() {
        let (batch, schema) = frames();
        let arguments = args("not a source map");
        let bind = BindParams {
            input_schema: Some(schema),
            arguments: arguments.clone(),
            ..Default::default()
        };
        let bound = LookupAll.on_bind(&bind).unwrap();
        let params = process_params(bound.output_schema, arguments);
        let out = LookupAll.process(&params, &batch).unwrap();
        let b = &out[0];
        let ids = b.column(0).as_primitive::<Int64Type>();
        assert_eq!(ids.value(1), 20, "id still passes through a bad map");
        for r in 0..3 {
            assert!(b.column(1).is_null(r), "bad map → NULL resolution");
        }
    }
}
