//! Node.js driver renderer.

use bson::Document;

use crate::codegen::Renderer;
use crate::codegen::escape::indent_lines;
use crate::codegen::js_ops::JsOps;
use crate::codegen::js_syntax::{JsDialect, JsSyntax, quote};
use crate::codegen::model::{CodeOptions, CodegenError};
use crate::store::{Namespace, Query};

/// Longest single-line `require` before the class list is wrapped.
const IMPORT_WIDTH: usize = 88;

pub(crate) struct Node;

impl Node {
    fn frame(ns: &Namespace, options: &CodeOptions, syn: &JsSyntax, body: &str) -> String {
        if !options.setup {
            return body.to_string();
        }
        let mut classes = vec!["MongoClient"];
        classes.extend(syn.used_classes());
        classes.sort_unstable();
        let (client, db, coll) = (
            &options.client_var,
            &options.db_var,
            &options.collection_var,
        );
        let inner = format!(
            "const {client} = new MongoClient(\"<your connection string>\");\ntry {{\n  await {client}.connect();\n  const {db} = {client}.db({});\n  const {coll} = {db}.collection({});\n\n{}\n}} finally {{\n  await {client}.close();\n}}",
            quote(&ns.db),
            quote(&ns.collection),
            indent_lines(body.trim_end(), "  ", 1),
        );
        format!(
            "{}\n\nasync function main() {{\n{}\n}}\n\nmain().catch(console.error);\n",
            Self::import_line(&classes),
            indent_lines(&inner, "  ", 1),
        )
    }

    /// The `require` of the driver classes, wrapped when it gets long.
    fn import_line(classes: &[&str]) -> String {
        let single = format!("const {{ {} }} = require(\"mongodb\");", classes.join(", "));
        if single.len() <= IMPORT_WIDTH {
            return single;
        }
        let names: String = classes.iter().map(|c| format!("  {c},\n")).collect();
        format!("const {{\n{names}}} = require(\"mongodb\");")
    }

    fn ops<'a>(syn: &'a JsSyntax, options: &'a CodeOptions) -> JsOps<'a> {
        JsOps {
            syn,
            collection: &options.collection_var,
            awaited: true,
            project_method: true,
            print: "console.log(results);",
        }
    }
}

impl Renderer for Node {
    fn find(
        &self,
        ns: &Namespace,
        query: &Query,
        options: &CodeOptions,
    ) -> Result<String, CodegenError> {
        let syn = JsSyntax::new(JsDialect::Node);
        let body = Self::ops(&syn, options).find(query)?;
        Ok(Self::frame(ns, options, &syn, &body))
    }

    fn aggregate(
        &self,
        ns: &Namespace,
        stages: &[Document],
        options: &CodeOptions,
    ) -> Result<String, CodegenError> {
        let syn = JsSyntax::new(JsDialect::Node);
        let body = Self::ops(&syn, options).aggregate(stages)?;
        Ok(Self::frame(ns, options, &syn, &body))
    }
}
