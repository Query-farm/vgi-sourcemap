//! `source_content(map, src_id) -> VARCHAR` — the inline `sourcesContent` of one
//! original source by id, or NULL if the map carries no embedded content for it.

use std::sync::Arc;

use arrow_array::builder::StringBuilder;
use arrow_array::{ArrayRef, RecordBatch};
use arrow_schema::DataType;
use vgi::{
    ArgSpec, BindParams, BindResponse, FunctionExample, FunctionMetadata, ProcessParams,
    ScalarFunction,
};
use vgi_rpc::{Result, RpcError};

use crate::arrow_io::int_val;
use crate::cache::MapCache;
use crate::maparg::map_bytes_cell;

pub struct SourceContent;

impl ScalarFunction for SourceContent {
    fn name(&self) -> &str {
        "source_content"
    }

    fn metadata(&self) -> FunctionMetadata {
        FunctionMetadata {
            description: "Return the inline sourcesContent of one original source (by src_id) \
                          embedded in the map, or NULL if absent"
                .into(),
            return_type: Some(DataType::Utf8),
            examples: vec![FunctionExample {
                sql: "SELECT sourcemap.main.source_content('{\"version\":3,\"sources\":[\"a.js\"],\
                      \"sourcesContent\":[\"const x = 1;\"],\"names\":[],\"mappings\":\"AAAA\"}', 0);"
                    .into(),
                description: "Fetch the embedded original source text for source 0.".into(),
                expected_output: None,
            }],
            tags: crate::meta::object_tags(
                "Inline Source Content",
                "Return the embedded original source text (the map's `sourcesContent`) for one \
                 source, identified by its 0-based src_id (matching the sources() table). The \
                 `map` argument is a path to a .map file, the raw map JSON text, or map BLOB \
                 bytes. Returns NULL when the map embeds no content for that id, or when the map \
                 is NULL/malformed.",
                "Return the inline `sourcesContent` for a source id, e.g. \
                 `source_content(read_text(map_path), 0)`. NULL when no inline content.",
                "source content, sourcesContent, inline source, original source text, embedded \
                 source, src_id",
            ),
            ..Default::default()
        }
    }

    fn argument_specs(&self) -> Vec<ArgSpec> {
        vec![
            ArgSpec::any_column(
                "map",
                0,
                "The source map: a path to a .map file, the raw map JSON text, or the map bytes.",
            ),
            ArgSpec::column_typed(
                "src_id",
                1,
                DataType::Int64,
                "The 0-based source id whose inline content to return (matches sources.src_id).",
            ),
        ]
    }

    fn on_bind(&self, _params: &BindParams) -> Result<BindResponse> {
        Ok(BindResponse::result(DataType::Utf8))
    }

    fn process(&self, params: &ProcessParams, batch: &RecordBatch) -> Result<RecordBatch> {
        let map = batch.column(0);
        let id = batch.column(1);
        let rows = batch.num_rows();
        let mut out = StringBuilder::new();
        let mut cache = MapCache::default();

        for i in 0..rows {
            let content = (|| {
                let bytes = match map_bytes_cell(map, i) {
                    Ok(Some(b)) => b,
                    _ => return None,
                };
                let src_id = u32::try_from(int_val(id, i).ok().flatten()?).ok()?;
                cache.get(&bytes)?.source_content(src_id)
            })();
            match content {
                Some(s) => out.append_value(s),
                None => out.append_null(),
            }
        }
        let arr: ArrayRef = Arc::new(out.finish());
        RecordBatch::try_new(params.output_schema.clone(), vec![arr])
            .map_err(|e| RpcError::runtime_error(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arrow_io::test_support::run_scalar_on;
    use arrow_array::cast::AsArray;
    use arrow_array::{Array, Int64Array, StringArray};
    use arrow_schema::Field;

    const MAP: &str = r#"{"version":3,"file":"o.js","sources":["a.js"],"sourcesContent":["export const x = 1;\n"],"names":[],"mappings":"AAAA"}"#;

    #[test]
    fn returns_inline_content_or_null() {
        let m: ArrayRef = Arc::new(StringArray::from(vec![Some(MAP), Some(MAP), None]));
        let id: ArrayRef = Arc::new(Int64Array::from(vec![Some(0), Some(9), Some(0)]));
        let out = run_scalar_on(
            &SourceContent,
            vec![m, id],
            vec![
                Field::new("map", DataType::Utf8, true),
                Field::new("src_id", DataType::Int64, true),
            ],
        )
        .unwrap();
        let s = out.as_string::<i32>();
        assert_eq!(s.value(0), "export const x = 1;\n");
        assert!(out.is_null(1), "no source 9 → NULL");
        assert!(out.is_null(2), "NULL map → NULL");
    }
}
