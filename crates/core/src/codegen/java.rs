//! Java sync driver renderer.

use bson::Document;

use crate::codegen::Renderer;
use crate::codegen::escape::indent_lines;
use crate::codegen::java_syntax::{JavaSyntax, quote};
use crate::codegen::model::{CodeOptions, CodegenError};
use crate::codegen::walk::{render_document, render_documents};
use crate::store::{Namespace, Query};

pub(crate) struct Java;

const CLIENT_IMPORTS: [&str; 6] = [
    "com.mongodb.client.MongoClient",
    "com.mongodb.client.MongoClients",
    "com.mongodb.client.MongoCollection",
    "com.mongodb.client.MongoDatabase",
    "java.util.ArrayList",
    "java.util.List",
];

impl Java {
    fn frame(ns: &Namespace, options: &CodeOptions, syn: &JavaSyntax, body: &str) -> String {
        if !options.setup {
            return body.to_string();
        }
        let mut imports: Vec<&str> = CLIENT_IMPORTS.to_vec();
        imports.push("org.bson.Document");
        imports.extend(syn.imports());
        imports.sort_unstable();
        imports.dedup();
        let lines: String = imports.iter().map(|i| format!("import {i};\n")).collect();
        let (client, db, coll) = (
            &options.client_var,
            &options.db_var,
            &options.collection_var,
        );
        let inner = format!(
            "MongoDatabase {db} = {client}.getDatabase({});\nMongoCollection<Document> {coll} = {db}.getCollection({});\n\n{}",
            quote(&ns.db),
            quote(&ns.collection),
            body.trim_end(),
        );
        format!(
            "{lines}\npublic class Main {{\n    public static void main(String[] args) {{\n        try (MongoClient {client} = MongoClients.create(\"<your connection string>\")) {{\n{}\n        }}\n    }}\n}}\n",
            indent_lines(&inner, "    ", 3),
        )
    }
}

/// `skip` / `limit` are `int` in the Java driver.
fn int_arg(name: &'static str, value: u64) -> Result<i32, CodegenError> {
    i32::try_from(value).map_err(|_| CodegenError::UnsupportedValue(name))
}

impl Renderer for Java {
    fn find(
        &self,
        ns: &Namespace,
        query: &Query,
        options: &CodeOptions,
    ) -> Result<String, CodegenError> {
        let syn = JavaSyntax::new();
        let mut body = format!(
            "Document filter = {};\n",
            render_document(&syn, &query.filter, 0)?
        );
        let mut chain: Vec<String> = Vec::new();
        if let Some(projection) = &query.projection {
            body.push_str(&format!(
                "Document projection = {};\n",
                render_document(&syn, projection, 0)?
            ));
            chain.push(".projection(projection)".into());
        }
        if let Some(sort) = &query.sort {
            body.push_str(&format!(
                "Document sort = {};\n",
                render_document(&syn, sort, 0)?
            ));
            chain.push(".sort(sort)".into());
        }
        if query.skip > 0 {
            chain.push(format!(".skip({})", int_arg("skip", query.skip)?));
        }
        if query.limit > 0 {
            chain.push(format!(".limit({})", int_arg("limit", query.limit)?));
        }
        chain.push(".into(new ArrayList<>())".into());
        let lines: String = chain.iter().map(|c| format!("\n    {c}")).collect();
        body.push_str(&format!(
            "\nList<Document> results = {}.find(filter){lines};\nSystem.out.println(results);\n",
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
        let syn = JavaSyntax::new();
        let pipeline = render_documents(&syn, stages, 0)?;
        let body = format!(
            "List<Document> pipeline = {pipeline};\n\nList<Document> results = {}.aggregate(pipeline).into(new ArrayList<>());\nSystem.out.println(results);\n",
            options.collection_var
        );
        Ok(Self::frame(ns, options, &syn, &body))
    }
}
