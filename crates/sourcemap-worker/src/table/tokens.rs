//! `tokens(map) -> (gen_line, gen_col, src_id, source, src_line, src_col, name)`
//! — explode a source map into its full decoded token (mapping) table, one row
//! per Base64-VLQ segment, in generated order. All line/column values are
//! 0-indexed (Source Map v3); `src_*` / `source` / `name` are NULL for a token
//! with no original-source information.

use std::sync::Arc;

use arrow_array::builder::{Int64Builder, StringBuilder};
use arrow_array::{ArrayRef, RecordBatch};
use arrow_schema::{DataType, Schema, SchemaRef};
use vgi::table_function::{TableFunction, TableProducer};
use vgi::{ArgSpec, BindParams, BindResponse, FunctionExample, FunctionMetadata, ProcessParams};
use vgi_rpc::{OutputCollector, Result, RpcError};

use crate::arrow_io::commented;

pub struct Tokens;

pub fn output_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        commented(
            "gen_line",
            DataType::Int64,
            false,
            "0-indexed generated (minified) line of this mapping.",
        ),
        commented(
            "gen_col",
            DataType::Int64,
            false,
            "0-indexed generated (minified) column of this mapping.",
        ),
        commented(
            "src_id",
            DataType::Int64,
            true,
            "0-based index into sources() of the original file, or NULL if the token has no source.",
        ),
        commented(
            "source",
            DataType::Utf8,
            true,
            "Original source file path this mapping points at, or NULL.",
        ),
        commented(
            "src_line",
            DataType::Int64,
            true,
            "0-indexed original source line, or NULL.",
        ),
        commented(
            "src_col",
            DataType::Int64,
            true,
            "0-indexed original source column, or NULL.",
        ),
        commented(
            "name",
            DataType::Utf8,
            true,
            "Original symbol name for this mapping, or NULL if none.",
        ),
    ]))
}

impl TableFunction for Tokens {
    fn name(&self) -> &str {
        "tokens"
    }

    fn metadata(&self) -> FunctionMetadata {
        let mut tags = crate::meta::object_tags(
            "Decode Source Map Tokens",
            "Decode a Source Map v3 (.map) into its full token table: one row per Base64-VLQ \
             mapping, in generated order, with columns gen_line, gen_col, src_id, source, \
             src_line, src_col, name. The `map` argument is a path to a .map file, the raw map \
             JSON text, or map `BLOB` bytes. Index maps (sections) are flattened first. All \
             line/column numbers are 0-indexed. Use it to inspect or join the raw mappings of a \
             source map.",
            "Explode a source map into its decoded token table (gen_line, gen_col, src_id, \
             source, src_line, src_col, name).",
            "tokens, mappings, decode source map, VLQ, segments, gen_line, gen_col, source map \
             table, explode mappings",
            "Inspection",
        );
        // VGI307/VGI321: static result schema (same columns for every call),
        // declared as structured `vgi.result_columns_schema` — one
        // {name,type,description} per returned column. Mirrors `output_schema()`.
        tags.push((
            "vgi.result_columns_schema".into(),
            r#"[
  {"name": "gen_line", "type": "BIGINT", "description": "0-indexed generated (minified) line of this mapping."},
  {"name": "gen_col", "type": "BIGINT", "description": "0-indexed generated (minified) column of this mapping."},
  {"name": "src_id", "type": "BIGINT", "description": "0-based index into sources() of the original file, or NULL if the token has no source."},
  {"name": "source", "type": "VARCHAR", "description": "Original source file path this mapping points at, or NULL."},
  {"name": "src_line", "type": "BIGINT", "description": "0-indexed original source line, or NULL."},
  {"name": "src_col", "type": "BIGINT", "description": "0-indexed original source column, or NULL."},
  {"name": "name", "type": "VARCHAR", "description": "Original symbol name for this mapping, or NULL if none."}
]"#
            .into(),
        ));
        let example_sql = format!(
            "SELECT gen_line, gen_col, source FROM sourcemap.main.tokens('{}');",
            crate::SAMPLE_MAP
        );
        const EXAMPLE_DESC: &str = "Explode an inline source map into its decoded token rows.";
        // VGI515: described-example carrier, byte-identical to the FunctionExample.
        tags.push((
            "vgi.example_queries".into(),
            crate::meta::example_queries_json(&[(EXAMPLE_DESC, &example_sql)]),
        ));
        FunctionMetadata {
            description: "Decode a source map into its full token (mapping) table".into(),
            examples: vec![FunctionExample {
                sql: example_sql,
                description: EXAMPLE_DESC.into(),
                expected_output: None,
            }],
            tags,
            ..Default::default()
        }
    }

    fn argument_specs(&self) -> Vec<ArgSpec> {
        vec![ArgSpec::const_arg(
            "map",
            0,
            "any",
            "The source map to decode: a path to a .map file, the raw map JSON text, or the map \
             bytes.",
        )]
    }

    fn on_bind(&self, _params: &BindParams) -> Result<BindResponse> {
        Ok(BindResponse {
            output_schema: output_schema(),
            opaque_data: Vec::new(),
        })
    }

    fn producer(&self, params: &ProcessParams) -> Result<Box<dyn TableProducer>> {
        // Decode eagerly so a bad map fails the scan up front.
        let smap = crate::table::load_const_map(&params.arguments)?;
        let rows = smap.tokens();
        Ok(Box::new(TokensProducer {
            schema: params.output_schema.clone(),
            rows: Some(rows),
        }))
    }
}

struct TokensProducer {
    schema: SchemaRef,
    rows: Option<Vec<crate::smap::TokenRow>>,
}

impl TableProducer for TokensProducer {
    fn next_batch(&mut self, _out: &mut OutputCollector) -> Result<Option<RecordBatch>> {
        let Some(rows) = self.rows.take() else {
            return Ok(None);
        };
        let mut gen_line = Int64Builder::new();
        let mut gen_col = Int64Builder::new();
        let mut src_id = Int64Builder::new();
        let mut source = StringBuilder::new();
        let mut src_line = Int64Builder::new();
        let mut src_col = Int64Builder::new();
        let mut name = StringBuilder::new();
        for r in &rows {
            gen_line.append_value(r.gen_line as i64);
            gen_col.append_value(r.gen_col as i64);
            match r.src_id {
                Some(v) => src_id.append_value(v as i64),
                None => src_id.append_null(),
            }
            match &r.source {
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
            match &r.name {
                Some(s) => name.append_value(s),
                None => name.append_null(),
            }
        }
        let cols: Vec<ArrayRef> = vec![
            Arc::new(gen_line.finish()),
            Arc::new(gen_col.finish()),
            Arc::new(src_id.finish()),
            Arc::new(source.finish()),
            Arc::new(src_line.finish()),
            Arc::new(src_col.finish()),
            Arc::new(name.finish()),
        ];
        Ok(Some(
            RecordBatch::try_new(self.schema.clone(), cols)
                .map_err(|e| RpcError::runtime_error(e.to_string()))?,
        ))
    }
}
