//! `sources(map) -> (src_id, source, has_content)` — list the original source
//! files a map references, with whether each one's content is embedded inline
//! (`sourcesContent`). Pair with the `source_content(map, src_id)` scalar to
//! fetch the embedded text.

use std::sync::Arc;

use arrow_array::builder::{BooleanBuilder, Int64Builder, StringBuilder};
use arrow_array::{ArrayRef, RecordBatch};
use arrow_schema::{DataType, Schema, SchemaRef};
use vgi::table_function::{TableFunction, TableProducer};
use vgi::{ArgSpec, BindParams, BindResponse, FunctionExample, FunctionMetadata, ProcessParams};
use vgi_rpc::{OutputCollector, Result, RpcError};

use crate::arrow_io::commented;

pub struct Sources;

pub fn output_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        commented(
            "src_id",
            DataType::Int64,
            false,
            "0-based source index (the id used by tokens.src_id and source_content).",
        ),
        commented(
            "source",
            DataType::Utf8,
            false,
            "Original source file path as recorded in the map's `sources`.",
        ),
        commented(
            "has_content",
            DataType::Boolean,
            false,
            "TRUE when the map embeds this source's text inline (`sourcesContent`).",
        ),
    ]))
}

impl TableFunction for Sources {
    fn name(&self) -> &str {
        "sources"
    }

    fn metadata(&self) -> FunctionMetadata {
        let mut tags = crate::meta::object_tags(
            "List Source Map Sources",
            "List the original source files referenced by a Source Map v3 (.map): one row per \
             source with its 0-based src_id, the source path, and has_content (whether the map \
             embeds that source's text inline via sourcesContent). The `map` argument is a path \
             to a .map file, the raw map JSON text, or map `BLOB` bytes. Use it to discover the \
             originals behind a bundle, then fetch embedded text with source_content(map, src_id).",
            "List a map's original sources (src_id, source, has_content).",
            "sources, source files, original files, sourcesContent, has_content, src_id, list \
             sources, bundle inputs",
            "Inspection",
        );
        // VGI307/VGI321: static result schema, declared as the structured
        // `vgi.result_columns_schema` (one {name,type,description} per column).
        tags.push((
            "vgi.result_columns_schema".into(),
            r#"[
  {"name": "src_id", "type": "BIGINT", "description": "0-based source index (feed it to source_content(map, src_id) to fetch this file's embedded text)."},
  {"name": "source", "type": "VARCHAR", "description": "Original source file path, with the map's sourceRoot already joined on."},
  {"name": "has_content", "type": "BOOLEAN", "description": "Whether the map embeds this source's text inline via sourcesContent."}
]"#
            .into(),
        ));
        let example_sql = format!(
            "SELECT src_id, source, has_content FROM sourcemap.main.sources('{}');",
            crate::SAMPLE_MAP
        );
        const EXAMPLE_DESC: &str = "List the original source files referenced by an inline map.";
        // VGI515: described-example carrier, byte-identical to the FunctionExample.
        tags.push((
            "vgi.example_queries".into(),
            crate::meta::example_queries_json(&[(EXAMPLE_DESC, &example_sql)]),
        ));
        FunctionMetadata {
            description: "List the original sources of a source map (with inline-content flag)"
                .into(),
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
            "The source map: a path to a .map file, the raw map JSON text, or the map bytes.",
        )]
    }

    fn on_bind(&self, _params: &BindParams) -> Result<BindResponse> {
        Ok(BindResponse {
            output_schema: output_schema(),
            opaque_data: Vec::new(),
        })
    }

    fn producer(&self, params: &ProcessParams) -> Result<Box<dyn TableProducer>> {
        let smap = crate::table::load_const_map(&params.arguments)?;
        Ok(Box::new(SourcesProducer {
            schema: params.output_schema.clone(),
            rows: Some(smap.sources()),
        }))
    }
}

struct SourcesProducer {
    schema: SchemaRef,
    rows: Option<Vec<crate::smap::SourceRow>>,
}

impl TableProducer for SourcesProducer {
    fn next_batch(&mut self, _out: &mut OutputCollector) -> Result<Option<RecordBatch>> {
        let Some(rows) = self.rows.take() else {
            return Ok(None);
        };
        let mut src_id = Int64Builder::new();
        let mut source = StringBuilder::new();
        let mut has_content = BooleanBuilder::new();
        for r in &rows {
            src_id.append_value(r.src_id as i64);
            source.append_value(&r.source);
            has_content.append_value(r.has_content);
        }
        let cols: Vec<ArrayRef> = vec![
            Arc::new(src_id.finish()),
            Arc::new(source.finish()),
            Arc::new(has_content.finish()),
        ];
        Ok(Some(
            RecordBatch::try_new(self.schema.clone(), cols)
                .map_err(|e| RpcError::runtime_error(e.to_string()))?,
        ))
    }
}
