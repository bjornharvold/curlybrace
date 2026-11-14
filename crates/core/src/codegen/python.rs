//! PyMongo renderer.

use bson::Document;

use crate::codegen::Renderer;
use crate::codegen::escape::indent;
use crate::codegen::model::{CodeOptions, CodegenError};
use crate::codegen::python_syntax::{PythonSyntax, quote};
use crate::codegen::walk::{Syntax, lay_out, render, render_document, render_documents};
use crate::store::{Namespace, Query};

pub(crate) struct Python;

impl Python {
    fn frame(ns: &Namespace, options: &CodeOptions, syn: &PythonSyntax, body: &str) -> String {
        if !options.setup {
            return body.to_string();
        }
        let mut head = String::new();
        let (plain, from): (Vec<&str>, Vec<&str>) = syn
            .import_lines()
            .into_iter()
            .partition(|l| l.starts_with("import "));
        for line in &plain {
            head.push_str(&format!("{line}\n"));
        }
        if !plain.is_empty() {
            head.push('\n');
        }
        for line in &from {
            head.push_str(&format!("{line}\n"));
        }
        head.push_str("from pymongo import MongoClient\n\n");
        let (client, db, coll) = (
            &options.client_var,
            &options.db_var,
            &options.collection_var,
        );
        format!(
            "{head}{client} = MongoClient(\"<your connection string>\")\n{db} = {client}[{}]\n{coll} = {db}[{}]\n\n{body}",
            quote(&ns.db),
            quote(&ns.collection),
        )
    }

    /// `sort` as PyMongo's list of `(field, direction)` pairs.
    fn sort_pairs(syn: &PythonSyntax, sort: &Document) -> Result<String, CodegenError> {
        let mut entries = Vec::with_capacity(sort.len());
        for (field, direction) in sort {
            entries.push(format!(
                "({}, {})",
                syn.key(field),
                render(syn, direction, 1)?
            ));
        }
        if entries.is_empty() {
            return Ok("[]".into());
        }
        Ok(lay_out(syn, ("[", "]"), "", &entries, 0))
    }
}

impl Renderer for Python {
    fn find(
        &self,
        ns: &Namespace,
        query: &Query,
        options: &CodeOptions,
    ) -> Result<String, CodegenError> {
        let syn = PythonSyntax::new();
        let mut body = format!("filter = {}\n", render_document(&syn, &query.filter, 0)?);
        let mut args: Vec<String> = vec!["filter".into()];
        if let Some(projection) = &query.projection {
            body.push_str(&format!(
                "projection = {}\n",
                render_document(&syn, projection, 0)?
            ));
            args.push("projection=projection".into());
        }
        if let Some(sort) = &query.sort {
            body.push_str(&format!("sort = {}\n", Self::sort_pairs(&syn, sort)?));
            args.push("sort=sort".into());
        }
        if query.skip > 0 {
            args.push(format!("skip={}", query.skip));
        }
        if query.limit > 0 {
            args.push(format!("limit={}", query.limit));
        }
        let coll = &options.collection_var;
        let call = if args.len() == 1 {
            format!("list({coll}.find(filter))")
        } else {
            let unit = indent("    ", 2);
            let lines: String = args.iter().map(|a| format!("{unit}{a},\n")).collect();
            format!("list(\n    {coll}.find(\n{lines}    )\n)")
        };
        body.push_str(&format!("\nresults = {call}\nprint(results)\n"));
        Ok(Self::frame(ns, options, &syn, &body))
    }

    fn aggregate(
        &self,
        ns: &Namespace,
        stages: &[Document],
        options: &CodeOptions,
    ) -> Result<String, CodegenError> {
        let syn = PythonSyntax::new();
        let pipeline = render_documents(&syn, stages, 0)?;
        let coll = &options.collection_var;
        let body = format!(
            "pipeline = {pipeline}\n\nresults = list({coll}.aggregate(pipeline))\nprint(results)\n"
        );
        Ok(Self::frame(ns, options, &syn, &body))
    }
}
