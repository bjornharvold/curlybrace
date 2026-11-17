//! Java sync driver renderer.

use bson::Document;

use crate::codegen::Renderer;
use crate::codegen::agg_options::{
    CodeAggOptions, alternate_words, case_first_words, mode_words, strength_words,
};
use crate::codegen::escape::indent_lines;
use crate::codegen::java_syntax::{JavaSyntax, quote};
use crate::codegen::model::{CodeOptions, CodegenError};
use crate::codegen::walk::{render_document, render_documents};
use crate::pipeline_options::{CollationOption, HintOption};
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
    i32::try_from(value).map_err(|_| CodegenError::OutOfRange(name))
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
        agg: &CodeAggOptions,
        options: &CodeOptions,
    ) -> Result<String, CodegenError> {
        let syn = JavaSyntax::new();
        let pipeline = render_documents(&syn, stages, 0)?;
        let handle = match agg.read_preference {
            Some(mode) => {
                syn.used("com.mongodb.ReadPreference");
                format!(
                    "{}.withReadPreference(ReadPreference.{}())",
                    options.collection_var,
                    mode_words(mode).camel
                )
            }
            None => options.collection_var.clone(),
        };
        let chain: String = Self::option_chain(&syn, agg)?
            .iter()
            .map(|setter| format!("\n    {setter}"))
            .collect();
        let body = format!(
            "List<Document> pipeline = {pipeline};\n\nList<Document> results = {handle}.aggregate(pipeline){chain}{}.into(new ArrayList<>());\nSystem.out.println(results);\n",
            if chain.is_empty() { "" } else { "\n    " }
        );
        let body = agg.with_note("//", body);
        Ok(Self::frame(ns, options, &syn, &body))
    }
}

impl Java {
    /// The setters on the `AggregateIterable`, in the order collation, hint, allowDiskUse,
    /// maxTimeMS, comment; a multi-line setter carries its own continuation indent.
    fn option_chain(syn: &JavaSyntax, agg: &CodeAggOptions) -> Result<Vec<String>, CodegenError> {
        let mut chain = Vec::new();
        if let Some(collation) = &agg.collation {
            chain.push(Self::collation(syn, collation));
        }
        match &agg.hint {
            Some(HintOption::Name(name)) => chain.push(format!(".hintString({})", quote(name))),
            Some(HintOption::Keys(keys)) => {
                chain.push(format!(".hint({})", render_document(syn, keys, 1)?));
            }
            None => {}
        }
        if let Some(on) = agg.allow_disk_use {
            chain.push(format!(".allowDiskUse({on})"));
        }
        if let Some(ms) = agg.max_time_ms {
            syn.used("java.util.concurrent.TimeUnit");
            chain.push(format!(".maxTime({ms}, TimeUnit.MILLISECONDS)"));
        }
        if let Some(comment) = &agg.comment {
            chain.push(format!(".comment({})", quote(comment)));
        }
        Ok(chain)
    }

    /// `.collation(Collation.builder()...build())`, one setter per line when there are several.
    fn collation(syn: &JavaSyntax, collation: &CollationOption) -> String {
        syn.used("com.mongodb.client.model.Collation");
        let mut setters = vec![format!(".locale({})", quote(&collation.locale))];
        if let Some(strength) = collation.strength {
            syn.used("com.mongodb.client.model.CollationStrength");
            let (_, word) = strength_words(strength);
            setters.push(format!(".collationStrength(CollationStrength.{word})"));
        }
        if let Some(on) = collation.case_level {
            setters.push(format!(".caseLevel({on})"));
        }
        if let Some(case_first) = collation.case_first {
            syn.used("com.mongodb.client.model.CollationCaseFirst");
            let (_, word) = case_first_words(case_first);
            setters.push(format!(".collationCaseFirst(CollationCaseFirst.{word})"));
        }
        if let Some(on) = collation.numeric_ordering {
            setters.push(format!(".numericOrdering({on})"));
        }
        if let Some(alternate) = collation.alternate {
            syn.used("com.mongodb.client.model.CollationAlternate");
            let (_, word) = alternate_words(alternate);
            setters.push(format!(".collationAlternate(CollationAlternate.{word})"));
        }
        if setters.len() == 1 {
            return format!(".collation(Collation.builder(){}.build())", setters[0]);
        }
        let lines: String = setters.iter().map(|s| format!("\n        {s}")).collect();
        format!(".collation(Collation.builder(){lines}\n        .build())")
    }
}
