//! PHP library renderer (`mongodb/mongodb`).

use bson::Document;

use crate::codegen::Renderer;
use crate::codegen::lossy::with_note;
use crate::codegen::model::{CodeOptions, CodegenError};
use crate::codegen::php_syntax::{PhpSyntax, quote};
use crate::codegen::walk::{render_document, render_documents};
use crate::store::{Namespace, Query};

pub(crate) struct Php;

/// PHP's superglobals; assigning one would clobber it.
const SUPERGLOBALS: [&str; 9] = [
    "GLOBALS", "_GET", "_POST", "_COOKIE", "_FILES", "_SERVER", "_ENV", "_REQUEST", "_SESSION",
];

fn check_names(options: &CodeOptions) -> Result<(), CodegenError> {
    for (which, name) in [
        ("client", &options.client_var),
        ("db", &options.db_var),
        ("collection", &options.collection_var),
    ] {
        if SUPERGLOBALS.contains(&name.as_str()) {
            return Err(CodegenError::InvalidName(which));
        }
    }
    Ok(())
}

impl Php {
    fn frame(ns: &Namespace, options: &CodeOptions, syn: &PhpSyntax, body: &str) -> String {
        let noted = with_note(syn, "//", body.to_string());
        let body = noted.as_str();
        if !options.setup {
            return noted;
        }
        let (client, db, coll) = (
            &options.client_var,
            &options.db_var,
            &options.collection_var,
        );
        format!(
            "<?php\n\nrequire 'vendor/autoload.php';\n\n${client} = new MongoDB\\Client('<your connection string>');\n${db} = ${client}->selectDatabase({});\n${coll} = ${db}->selectCollection({});\n\n{body}",
            quote(&ns.db),
            quote(&ns.collection),
        )
    }
}

impl Renderer for Php {
    fn find(
        &self,
        ns: &Namespace,
        query: &Query,
        options: &CodeOptions,
    ) -> Result<String, CodegenError> {
        check_names(options)?;
        let syn = PhpSyntax::new();
        let mut body = format!("$filter = {};\n", render_document(&syn, &query.filter, 0)?);
        let mut args: Vec<String> = Vec::new();
        if let Some(projection) = &query.projection {
            body.push_str(&format!(
                "$projection = {};\n",
                render_document(&syn, projection, 0)?
            ));
            args.push("'projection' => $projection".into());
        }
        if let Some(sort) = &query.sort {
            body.push_str(&format!("$sort = {};\n", render_document(&syn, sort, 0)?));
            args.push("'sort' => $sort".into());
        }
        if query.skip > 0 {
            args.push(format!("'skip' => {}", query.skip));
        }
        if query.limit > 0 {
            args.push(format!("'limit' => {}", query.limit));
        }
        let coll = &options.collection_var;
        let call = if args.is_empty() {
            format!("${coll}->find($filter)->toArray()")
        } else {
            let lines: String = args.iter().map(|a| format!("    {a},\n")).collect();
            format!("${coll}->find($filter, [\n{lines}])->toArray()")
        };
        body.push_str(&format!("\n$results = {call};\nprint_r($results);\n"));
        Ok(Self::frame(ns, options, &syn, &body))
    }

    fn aggregate(
        &self,
        ns: &Namespace,
        stages: &[Document],
        options: &CodeOptions,
    ) -> Result<String, CodegenError> {
        check_names(options)?;
        let syn = PhpSyntax::new();
        let pipeline = render_documents(&syn, stages, 0)?;
        let body = format!(
            "$pipeline = {pipeline};\n\n$results = ${}->aggregate($pipeline)->toArray();\nprint_r($results);\n",
            options.collection_var
        );
        Ok(Self::frame(ns, options, &syn, &body))
    }
}
