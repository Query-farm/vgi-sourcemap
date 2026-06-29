//! `meta(map) -> (version, file, source_root, n_sources, n_names, n_mappings)` —
//! one row of map-level metadata. Counts are over the flattened token list, so
//! an index map (`sections`) reports its combined totals.

use std::sync::Arc;

use arrow_array::builder::{Int64Builder, StringBuilder};
use arrow_array::{ArrayRef, RecordBatch};
use arrow_schema::{DataType, Schema, SchemaRef};
use vgi::table_function::{TableFunction, TableProducer};
use vgi::{ArgSpec, BindParams, BindResponse, FunctionMetadata, ProcessParams};
use vgi_rpc::{OutputCollector, Result, RpcError};

use crate::arrow_io::commented;

pub struct Meta;

pub fn output_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        commented(
            "version",
            DataType::Int64,
            false,
            "Source map spec version (always 3 — this worker decodes v3 maps).",
        ),
        commented(
            "file",
            DataType::Utf8,
            true,
            "The generated file the map describes (`file`), or NULL if unset.",
        ),
        commented(
            "source_root",
            DataType::Utf8,
            true,
            "The `sourceRoot` prepended to source paths, or NULL if unset.",
        ),
        commented(
            "n_sources",
            DataType::Int64,
            false,
            "Number of original source files referenced.",
        ),
        commented(
            "n_names",
            DataType::Int64,
            false,
            "Number of original symbol names in the map's `names` table.",
        ),
        commented(
            "n_mappings",
            DataType::Int64,
            false,
            "Number of decoded tokens (mappings) after flattening any sections.",
        ),
    ]))
}

impl TableFunction for Meta {
    fn name(&self) -> &str {
        "meta"
    }

    fn metadata(&self) -> FunctionMetadata {
        let mut tags = crate::meta::object_tags(
            "Source Map Metadata",
            "Return one row of metadata about a Source Map v3 (.map): version, file (the \
             generated file it describes), source_root, n_sources, n_names, and n_mappings (the \
             decoded token count after flattening any sections). The `map` argument is a path to \
             a .map file, the raw map JSON text, or map BLOB bytes. Use it to size or sanity-check \
             a map before decoding it in full.",
            "Get one row of map-level metadata (version, file, source_root, n_sources, n_names, \
             n_mappings), e.g. `SELECT * FROM meta('/dist/app.min.js.map')`.",
            "meta, metadata, source map version, file, sourceRoot, source_root, counts, n_sources, \
             n_mappings, summary",
        );
        tags.push((
            "vgi.result_columns_md".into(),
            "| column | type | description |\n\
             |---|---|---|\n\
             | `version` | BIGINT | spec version (3). |\n\
             | `file` | VARCHAR | generated file (NULL if unset). |\n\
             | `source_root` | VARCHAR | `sourceRoot` (NULL if unset). |\n\
             | `n_sources` | BIGINT | original source count. |\n\
             | `n_names` | BIGINT | symbol-name count. |\n\
             | `n_mappings` | BIGINT | decoded token count. |"
                .into(),
        ));
        FunctionMetadata {
            description: "Return one row of source-map metadata (version, file, counts)".into(),
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
        Ok(Box::new(MetaProducer {
            schema: params.output_schema.clone(),
            meta: Some(smap.meta()),
        }))
    }
}

struct MetaProducer {
    schema: SchemaRef,
    meta: Option<crate::smap::Meta>,
}

impl TableProducer for MetaProducer {
    fn next_batch(&mut self, _out: &mut OutputCollector) -> Result<Option<RecordBatch>> {
        let Some(m) = self.meta.take() else {
            return Ok(None);
        };
        let mut version = Int64Builder::new();
        let mut file = StringBuilder::new();
        let mut source_root = StringBuilder::new();
        let mut n_sources = Int64Builder::new();
        let mut n_names = Int64Builder::new();
        let mut n_mappings = Int64Builder::new();

        version.append_value(m.version as i64);
        match &m.file {
            Some(s) => file.append_value(s),
            None => file.append_null(),
        }
        match &m.source_root {
            Some(s) => source_root.append_value(s),
            None => source_root.append_null(),
        }
        n_sources.append_value(m.n_sources as i64);
        n_names.append_value(m.n_names as i64);
        n_mappings.append_value(m.n_mappings as i64);

        let cols: Vec<ArrayRef> = vec![
            Arc::new(version.finish()),
            Arc::new(file.finish()),
            Arc::new(source_root.finish()),
            Arc::new(n_sources.finish()),
            Arc::new(n_names.finish()),
            Arc::new(n_mappings.finish()),
        ];
        Ok(Some(
            RecordBatch::try_new(self.schema.clone(), cols)
                .map_err(|e| RpcError::runtime_error(e.to_string()))?,
        ))
    }
}
