//! PyMongo renderer.

use bson::Document;

use crate::codegen::Renderer;
use crate::codegen::agg_options::{CodeAggOptions, mode_upper_snake, plain_entries};
use crate::codegen::escape::indent;
use crate::codegen::lossy::with_note;
use crate::codegen::model::{CodeOptions, CodegenError};
use crate::codegen::python_syntax::{PythonSyntax, quote};
use crate::codegen::range::i64_arg;
use crate::codegen::walk::{Syntax, lay_out, render, render_document, render_documents};
use crate::store::{Namespace, Query};

pub(crate) struct Python;

impl Python {
    /// `read_preference`: the code names `ReadPreference`, so the import line lists it.
    fn frame(
        ns: &Namespace,
        options: &CodeOptions,
        syn: &PythonSyntax,
        body: &str,
        read_preference: bool,
    ) -> String {
        let noted = with_note(syn, "#", body.to_string());
        let body = noted.as_str();
        if !options.setup {
            return noted;
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
        let pymongo = if read_preference {
            "MongoClient, ReadPreference"
        } else {
            "MongoClient"
        };
        head.push_str(&format!("from pymongo import {pymongo}\n\n"));
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

    /// `list(collection.aggregate(pipeline, ..))`: options are keyword arguments, one per line
    /// (the layout of `find`); the read preference is set on a handle derived in the call.
    fn aggregate_call(
        syn: &PythonSyntax,
        agg: &CodeAggOptions,
        coll: &str,
    ) -> Result<String, CodegenError> {
        let handle = match agg.read_preference {
            Some(mode) => format!(
                "{coll}.with_options(read_preference=ReadPreference.{})",
                mode_upper_snake(mode)
            ),
            None => coll.to_string(),
        };
        let hint_keys = |keys: &Document| Self::sort_pairs(syn, keys);
        let entries = plain_entries(syn, agg, |name| name.to_string(), 2, Some(&hint_keys))?;
        if entries.is_empty() {
            return Ok(match agg.read_preference {
                Some(_) => format!("list(\n    {handle}.aggregate(pipeline)\n)"),
                None => format!("list({handle}.aggregate(pipeline))"),
            });
        }
        let unit = indent("    ", 2);
        let lines: String = entries
            .iter()
            .map(|(name, value)| format!("{unit}{name}={value},\n"))
            .collect();
        Ok(format!(
            "list(\n    {handle}.aggregate(\n{unit}pipeline,\n{lines}    )\n)"
        ))
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
            args.push(format!("skip={}", i64_arg("skip", query.skip)?));
        }
        if query.limit > 0 {
            args.push(format!("limit={}", i64_arg("limit", query.limit)?));
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
        Ok(Self::frame(ns, options, &syn, &body, false))
    }

    fn aggregate(
        &self,
        ns: &Namespace,
        stages: &[Document],
        agg: &CodeAggOptions,
        options: &CodeOptions,
    ) -> Result<String, CodegenError> {
        let syn = PythonSyntax::new();
        let pipeline = render_documents(&syn, stages, 0)?;
        let call = Self::aggregate_call(&syn, agg, &options.collection_var)?;
        let body = format!("pipeline = {pipeline}\n\nresults = {call}\nprint(results)\n");
        let body = agg.with_note("#", body);
        Ok(Self::frame(
            ns,
            options,
            &syn,
            &body,
            agg.read_preference.is_some(),
        ))
    }
}
