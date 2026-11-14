//! Rust driver (`mongodb` 3, async) renderer.

use bson::Document;

use crate::codegen::Renderer;
use crate::codegen::escape::indent_lines;
use crate::codegen::model::{CodeOptions, CodegenError};
use crate::codegen::rust_syntax::{RustSyntax, quote};
use crate::codegen::walk::{render_document, render_documents};
use crate::store::{Namespace, Query};

pub(crate) struct Rust;

impl Rust {
    fn frame(ns: &Namespace, options: &CodeOptions, syn: &RustSyntax, body: &str) -> String {
        let limit = syn.recursion_limit();
        if !options.setup {
            return match limit {
                // The attribute is only legal at the crate root.
                Some(n) => {
                    format!("// Needs #![recursion_limit = \"{n}\"] at the top of main.rs\n{body}")
                }
                None => body.to_string(),
            };
        }
        let attribute = limit
            .map(|n| format!("#![recursion_limit = \"{n}\"]\n\n"))
            .unwrap_or_default();
        let bson_import = if syn.uses_bson_path() {
            "{self, Document, doc}"
        } else {
            "{Document, doc}"
        };
        let (client, db, coll) = (
            &options.client_var,
            &options.db_var,
            &options.collection_var,
        );
        let inner = format!(
            "let {client} = Client::with_uri_str(\"<your connection string>\").await?;\nlet {db} = {client}.database({});\nlet {coll} = {db}.collection::<Document>({});\n\n{}\n\nOk(())",
            quote(&ns.db),
            quote(&ns.collection),
            body.trim_end(),
        );
        format!(
            "{attribute}use futures::TryStreamExt;\nuse mongodb::Client;\nuse mongodb::bson::{bson_import};\n\n#[tokio::main]\nasync fn main() -> Result<(), Box<dyn std::error::Error>> {{\n{}\n}}\n",
            indent_lines(&inner, "    ", 1),
        )
    }
}

const PRINT_CURSOR: &str =
    "while let Some(item) = cursor.try_next().await? {\n    println!(\"{item}\");\n}\n";

/// `limit` is an `i64` in the Rust driver.
fn limit_arg(value: u64) -> Result<i64, CodegenError> {
    i64::try_from(value).map_err(|_| CodegenError::UnsupportedValue("limit"))
}

impl Renderer for Rust {
    fn find(
        &self,
        ns: &Namespace,
        query: &Query,
        options: &CodeOptions,
    ) -> Result<String, CodegenError> {
        let syn = RustSyntax::new();
        let mut body = format!(
            "let filter = doc! {};\n",
            render_document(&syn, &query.filter, 0)?
        );
        let mut chain: Vec<String> = Vec::new();
        if let Some(projection) = &query.projection {
            body.push_str(&format!(
                "let projection = doc! {};\n",
                render_document(&syn, projection, 0)?
            ));
            chain.push(".projection(projection)".into());
        }
        if let Some(sort) = &query.sort {
            body.push_str(&format!(
                "let sort = doc! {};\n",
                render_document(&syn, sort, 0)?
            ));
            chain.push(".sort(sort)".into());
        }
        if query.skip > 0 {
            chain.push(format!(".skip({})", query.skip));
        }
        if query.limit > 0 {
            chain.push(format!(".limit({})", limit_arg(query.limit)?));
        }
        let lines: String = chain.iter().map(|c| format!("\n    {c}")).collect();
        body.push_str(&format!(
            "\nlet mut cursor = {}\n    .find(filter){lines}\n    .await?;\n{PRINT_CURSOR}",
            options.collection_var
        ));
        Ok(Self::frame(ns, options, &syn, &body))
    }

    fn aggregate(
        &self,
        ns: &Namespace,
        stages: &[Document],
        options: &CodeOptions,
    ) -> Result<String, CodegenError> {
        let syn = RustSyntax::new();
        let pipeline = render_documents(&syn, stages, 0)?;
        let body = format!(
            "let pipeline = {pipeline};\n\nlet mut cursor = {}.aggregate(pipeline).await?;\n{PRINT_CURSOR}",
            options.collection_var
        );
        Ok(Self::frame(ns, options, &syn, &body))
    }
}
