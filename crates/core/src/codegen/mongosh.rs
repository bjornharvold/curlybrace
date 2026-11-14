//! mongosh script renderer.

use bson::Document;

use crate::codegen::Renderer;
use crate::codegen::js_ops::JsOps;
use crate::codegen::js_syntax::{JsDialect, JsSyntax, quote};
use crate::codegen::lossy::with_note;
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

    /// The body with its conversion note, once the shell is known to keep the field order. The
    /// shell lists integer-like keys first and has no ordered literal, so a document whose keys
    /// are in another order is refused rather than written with its fields moved.
    fn finish(syn: &JsSyntax, body: String) -> Result<String, CodegenError> {
        match syn.lost_key_order() {
            true => Err(CodegenError::KeyOrder),
            false => Ok(with_note(syn, "//", body)),
        }
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
        Ok(Self::frame(ns, options, Self::finish(&syn, body)?))
    }

    fn aggregate(
        &self,
        ns: &Namespace,
        stages: &[Document],
        options: &CodeOptions,
    ) -> Result<String, CodegenError> {
        let syn = JsSyntax::new(JsDialect::Mongosh);
        let body = Self::ops(&syn, options).aggregate(stages)?;
        Ok(Self::frame(ns, options, Self::finish(&syn, body)?))
    }
}
