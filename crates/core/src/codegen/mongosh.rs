//! mongosh script renderer.

use bson::Document;

use crate::codegen::Renderer;
use crate::codegen::js_ops::JsOps;
use crate::codegen::js_syntax::{JsDialect, JsSyntax, quote};
use crate::codegen::model::{CodeOptions, CodegenError};
use crate::store::{Namespace, Query};

pub(crate) struct Mongosh;

impl Mongosh {
    fn frame(ns: &Namespace, options: &CodeOptions, body: String) -> String {
        if !options.setup {
            return body;
        }
        format!(
            "// Run with: mongosh \"<your connection string>\"\nconst {} = db.getSiblingDB({}).getCollection({});\n\n{body}",
            options.collection_var,
            quote(&ns.db),
            quote(&ns.collection),
        )
    }

    fn ops<'a>(syn: &'a JsSyntax, options: &'a CodeOptions) -> JsOps<'a> {
        JsOps {
            syn,
            collection: &options.collection_var,
            awaited: false,
            project_method: false,
            print: "printjson(results);",
        }
    }
}

impl Renderer for Mongosh {
    fn find(
        &self,
        ns: &Namespace,
        query: &Query,
        options: &CodeOptions,
    ) -> Result<String, CodegenError> {
        let syn = JsSyntax::new(JsDialect::Mongosh);
        let body = Self::ops(&syn, options).find(query)?;
        Ok(Self::frame(ns, options, body))
    }

    fn aggregate(
        &self,
        ns: &Namespace,
        stages: &[Document],
        options: &CodeOptions,
    ) -> Result<String, CodegenError> {
        let syn = JsSyntax::new(JsDialect::Mongosh);
        let body = Self::ops(&syn, options).aggregate(stages)?;
        Ok(Self::frame(ns, options, body))
    }
}
