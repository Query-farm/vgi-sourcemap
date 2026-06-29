//! Small Arrow helpers shared across the scalar / table / table-in-out adapters:
//! reading VARCHAR and integer input cells, and the `STRUCT(source, src_line,
//! src_col, name)` output type that `lookup` publishes at bind and builds at
//! process. A `#[cfg(test)]` harness drives a function end-to-end in-process.

use arrow_array::cast::AsArray;
use arrow_array::types::{
    Decimal128Type, Float32Type, Float64Type, Int16Type, Int32Type, Int64Type, Int8Type,
    UInt16Type, UInt32Type, UInt64Type, UInt8Type,
};
use arrow_array::{Array, ArrayRef};
use arrow_schema::{DataType, Field, Fields};
use vgi_rpc::{Result, RpcError};

/// Read element `row` of an integer-ish column as `i64`, or `None` if null.
/// Accepts any of DuckDB's integer widths plus floats/decimals (a literal like
/// `lookup(map, 10, 5)` may arrive as INTEGER, and a computed position as
/// DOUBLE/DECIMAL); fractional values are truncated toward zero. Errors on a
/// non-numeric column.
pub fn int_val(col: &ArrayRef, row: usize) -> Result<Option<i64>> {
    if col.is_null(row) {
        return Ok(None);
    }
    Ok(Some(match col.data_type() {
        DataType::Int64 => col.as_primitive::<Int64Type>().value(row),
        DataType::Int32 => col.as_primitive::<Int32Type>().value(row) as i64,
        DataType::Int16 => col.as_primitive::<Int16Type>().value(row) as i64,
        DataType::Int8 => col.as_primitive::<Int8Type>().value(row) as i64,
        DataType::UInt64 => col.as_primitive::<UInt64Type>().value(row) as i64,
        DataType::UInt32 => col.as_primitive::<UInt32Type>().value(row) as i64,
        DataType::UInt16 => col.as_primitive::<UInt16Type>().value(row) as i64,
        DataType::UInt8 => col.as_primitive::<UInt8Type>().value(row) as i64,
        DataType::Float64 => col.as_primitive::<Float64Type>().value(row) as i64,
        DataType::Float32 => col.as_primitive::<Float32Type>().value(row) as i64,
        DataType::Decimal128(_, scale) => {
            let raw = col.as_primitive::<Decimal128Type>().value(row);
            (raw / 10i128.pow((*scale).max(0) as u32)) as i64
        }
        other => {
            return Err(RpcError::value_error(format!(
                "expected an integer (BIGINT) argument, got {other:?}"
            )))
        }
    }))
}

/// Read a generated line/column cell as a non-negative `u32`, or `None` if null
/// or negative (a negative position can never match a token → unresolvable).
pub fn pos_u32(col: &ArrayRef, row: usize) -> Result<Option<u32>> {
    Ok(int_val(col, row)?.and_then(|v| u32::try_from(v).ok()))
}

/// The fixed `STRUCT(source VARCHAR, src_line BIGINT, src_col BIGINT, name
/// VARCHAR)` fields that `lookup` returns — shared so `on_bind` and `process`
/// agree exactly. All fields are nullable (a no-match, or a token without a
/// source/name, yields NULLs).
pub fn resolved_struct_fields() -> Fields {
    Fields::from(vec![
        Field::new("source", DataType::Utf8, true),
        Field::new("src_line", DataType::Int64, true),
        Field::new("src_col", DataType::Int64, true),
        Field::new("name", DataType::Utf8, true),
    ])
}

/// A column field carrying a `comment` (surfaced via `duckdb_columns().comment`).
pub fn commented(name: &str, ty: DataType, nullable: bool, comment: &str) -> Field {
    Field::new(name, ty, nullable).with_metadata(std::collections::HashMap::from([(
        "comment".to_string(),
        comment.to_string(),
    )]))
}

#[cfg(test)]
pub mod test_support {
    use std::sync::Arc;

    use arrow_array::{ArrayRef, RecordBatch};
    use arrow_schema::{Schema, SchemaRef};
    use vgi::arguments::Arguments;
    use vgi::{BindParams, ProcessParams, ScalarFunction};
    use vgi_rpc::Result;

    /// Build a `ProcessParams` carrying the given output schema and arguments.
    pub fn process_params(output_schema: SchemaRef, arguments: Arguments) -> ProcessParams {
        ProcessParams {
            output_schema,
            input_schema: None,
            execution_id: Vec::new(),
            init_opaque_data: Vec::new(),
            arguments,
            settings: Default::default(),
            secrets: Default::default(),
            auth_principal: None,
            projection_ids: None,
            pushdown_filters: None,
            join_keys: Vec::new(),
            storage: None,
            order_by_column: None,
            order_by_direction: None,
            order_by_null_order: None,
            order_by_limit: None,
            tablesample_percentage: None,
            tablesample_seed: None,
            attach_opaque_data: None,
            at_unit: None,
            at_value: None,
            copy_from: None,
        }
    }

    /// Run a scalar function over a prebuilt input batch: `on_bind` then
    /// `process`, returning the single result column.
    pub fn run_scalar_on<F: ScalarFunction>(
        f: &F,
        cols: Vec<ArrayRef>,
        fields: Vec<arrow_schema::Field>,
    ) -> Result<ArrayRef> {
        let schema = Arc::new(Schema::new(fields));
        let batch = RecordBatch::try_new(schema.clone(), cols).unwrap();
        let bind = BindParams {
            input_schema: Some(schema),
            ..Default::default()
        };
        let bound = f.on_bind(&bind)?;
        let params = process_params(bound.output_schema.clone(), Arguments::default());
        Ok(f.process(&params, &batch)?.column(0).clone())
    }
}
