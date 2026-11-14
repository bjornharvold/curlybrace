//! Ruby driver renderer.
//!
//! The snippet is top-level code. Names starting with an uppercase letter are
//! constants in Ruby (they would also shadow `Mongo`, `Time`, ...), so they
//! are refused rather than silently emitted.

use bson::Document;

use crate::codegen::Renderer;
use crate::codegen::model::{CodeOptions, CodegenError};
use crate::codegen::ruby_syntax::{RubySyntax, quote};
use crate::codegen::walk::{render_document, render_documents};
use crate::store::{Namespace, Query};

pub(crate) struct Ruby;

fn check_names(options: &CodeOptions) -> Result<(), CodegenError> {
    for (which, name) in [
        ("client", &options.client_var),
        ("db", &options.db_var),
        ("collection", &options.collection_var),
    ] {
        if name.starts_with(|c: char| c.is_ascii_uppercase()) {
            return Err(CodegenError::InvalidName(which));
        }
    }
    Ok(())
}

impl Ruby {
    fn frame(ns: &Namespace, options: &CodeOptions, body: &str) -> String {
        if !options.setup {
            return body.to_string();
        }
        let (client, db, coll) = (
            &options.client_var,
            &options.db_var,
            &options.collection_var,
        );
        format!(
            "require 'mongo'\n\n{client} = Mongo::Client.new('<your connection string>')\n{db} = {client}.use({}).database\n{coll} = {db}[{}]\n\n{body}",
            quote(&ns.db),
            quote(&ns.collection),
        )
    }
}

impl Renderer for Ruby {
    fn find(
        &self,
        ns: &Namespace,
        query: &Query,
        options: &CodeOptions,
    ) -> Result<String, CodegenError> {
        check_names(options)?;
        let syn = RubySyntax;
        let mut body = format!("filter = {}\n", render_document(&syn, &query.filter, 0)?);
        let mut args: Vec<String> = Vec::new();
        if let Some(projection) = &query.projection {
            body.push_str(&format!(
                "projection = {}\n",
                render_document(&syn, projection, 0)?
            ));
            args.push("projection: projection".into());
        }
        if let Some(sort) = &query.sort {
            body.push_str(&format!("sort = {}\n", render_document(&syn, sort, 0)?));
            args.push("sort: sort".into());
        }
        if query.skip > 0 {
            args.push(format!("skip: {}", query.skip));
        }
        if query.limit > 0 {
            args.push(format!("limit: {}", query.limit));
        }
        let coll = &options.collection_var;
        let call = if args.is_empty() {
            format!("{coll}.find(filter).to_a")
        } else {
            let lines: Vec<String> = args.iter().map(|a| format!("  {a}")).collect();
            format!("{coll}.find(\n  filter,\n{}\n).to_a", lines.join(",\n"))
        };
        body.push_str(&format!("\nresults = {call}\nputs results.inspect\n"));
        Ok(Self::frame(ns, options, &body))
    }

    fn aggregate(
        &self,
        ns: &Namespace,
        stages: &[Document],
        options: &CodeOptions,
    ) -> Result<String, CodegenError> {
        check_names(options)?;
        let pipeline = render_documents(&RubySyntax, stages, 0)?;
        let body = format!(
            "pipeline = {pipeline}\n\nresults = {}.aggregate(pipeline).to_a\nputs results.inspect\n",
            options.collection_var
        );
        Ok(Self::frame(ns, options, &body))
    }
}
