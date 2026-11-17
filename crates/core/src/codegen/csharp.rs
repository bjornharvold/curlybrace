//! .NET driver renderer (C# top-level statements).

use bson::Document;

use crate::codegen::Renderer;
use crate::codegen::agg_options::{
    CodeAggOptions, alternate_words, case_first_words, mode_words, strength_words,
};
use crate::codegen::csharp_syntax::{CsharpSyntax, quote};
use crate::codegen::lossy::with_note;
use crate::codegen::model::{CodeOptions, CodegenError};
use crate::codegen::walk::{render, render_document, render_documents};
use crate::pipeline_options::CollationOption;
use crate::store::{Namespace, Query};

pub(crate) struct Csharp;

impl Csharp {
    fn frame(ns: &Namespace, options: &CodeOptions, syn: &CsharpSyntax, body: &str) -> String {
        let body = with_note(syn, "//", body.to_string());
        if !options.setup {
            return body;
        }
        let (client, db, coll) = (
            &options.client_var,
            &options.db_var,
            &options.collection_var,
        );
        format!(
            "using MongoDB.Bson;\nusing MongoDB.Driver;\n\nvar {client} = new MongoClient(\"<your connection string>\");\nvar {db} = {client}.GetDatabase({});\nvar {coll} = {db}.GetCollection<BsonDocument>({});\n\n{body}",
            quote(&ns.db),
            quote(&ns.collection),
        )
    }
}

/// `Skip` / `Limit` take `int?` in the .NET driver.
fn int_arg(name: &'static str, value: u64) -> Result<i32, CodegenError> {
    i32::try_from(value).map_err(|_| CodegenError::OutOfRange(name))
}

impl Renderer for Csharp {
    fn find(
        &self,
        ns: &Namespace,
        query: &Query,
        _agg: &CodeAggOptions,
        options: &CodeOptions,
    ) -> Result<String, CodegenError> {
        let syn = CsharpSyntax::new();
        let mut body = format!(
            "var filter = {};\n",
            render_document(&syn, &query.filter, 0)?
        );
        let mut chain: Vec<String> = Vec::new();
        if let Some(projection) = &query.projection {
            body.push_str(&format!(
                "var projection = {};\n",
                render_document(&syn, projection, 0)?
            ));
            chain.push(".Project<BsonDocument>(projection)".into());
        }
        if let Some(sort) = &query.sort {
            body.push_str(&format!(
                "var sort = {};\n",
                render_document(&syn, sort, 0)?
            ));
            chain.push(".Sort(sort)".into());
        }
        if query.skip > 0 {
            chain.push(format!(".Skip({})", int_arg("skip", query.skip)?));
        }
        if query.limit > 0 {
            chain.push(format!(".Limit({})", int_arg("limit", query.limit)?));
        }
        chain.push(".ToList()".into());
        let lines: String = chain.iter().map(|c| format!("\n    {c}")).collect();
        body.push_str(&format!(
            "\nvar results = {}.Find(filter){lines};\nresults.ForEach(Console.WriteLine);\n",
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
        let syn = CsharpSyntax::new();
        let pipeline = render_documents(&syn, stages, 0)?;
        let handle = match agg.read_preference {
            Some(mode) => format!(
                "{}.WithReadPreference(ReadPreference.{})",
                options.collection_var,
                mode_words(mode).pascal
            ),
            None => options.collection_var.clone(),
        };
        let entries = Self::option_entries(&syn, agg)?;
        let (declaration, argument) = if entries.is_empty() {
            (String::new(), "")
        } else {
            let lines: String = entries.iter().map(|e| format!("    {e},\n")).collect();
            (
                format!("var options = new AggregateOptions\n{{\n{lines}}};\n\n"),
                ", options",
            )
        };
        let body = format!(
            "var pipeline = {pipeline};\n\n{declaration}var results = {handle}.Aggregate<BsonDocument>(pipeline{argument}).ToList();\nresults.ForEach(Console.WriteLine);\n"
        );
        let body = agg.with_note("//", body);
        Ok(Self::frame(ns, options, &syn, &body))
    }
}

impl Csharp {
    /// The property initializers of `AggregateOptions`, in the order collation, hint,
    /// allowDiskUse, maxTimeMS, comment.
    fn option_entries(
        syn: &CsharpSyntax,
        agg: &CodeAggOptions,
    ) -> Result<Vec<String>, CodegenError> {
        let mut entries = Vec::new();
        if let Some(collation) = &agg.collation {
            entries.push(format!("Collation = {}", Self::collation(collation)));
        }
        if let Some(hint) = &agg.hint {
            entries.push(format!("Hint = {}", render(syn, &hint.to_bson(), 1)?));
        }
        if let Some(on) = agg.allow_disk_use {
            entries.push(format!("AllowDiskUse = {on}"));
        }
        if let Some(ms) = agg.max_time_ms {
            entries.push(format!("MaxTime = TimeSpan.FromMilliseconds({ms})"));
        }
        if let Some(comment) = &agg.comment {
            entries.push(format!("Comment = {}", quote(comment)));
        }
        Ok(entries)
    }

    /// `new Collation("en", caseLevel: .., ..)` with the constructor's parameter names.
    fn collation(collation: &CollationOption) -> String {
        let mut args = vec![quote(&collation.locale)];
        if let Some(on) = collation.case_level {
            args.push(format!("caseLevel: {on}"));
        }
        if let Some(case_first) = collation.case_first {
            let (word, _) = case_first_words(case_first);
            args.push(format!("caseFirst: CollationCaseFirst.{word}"));
        }
        if let Some(strength) = collation.strength {
            let (word, _) = strength_words(strength);
            args.push(format!("strength: CollationStrength.{word}"));
        }
        if let Some(on) = collation.numeric_ordering {
            args.push(format!("numericOrdering: {on}"));
        }
        if let Some(alternate) = collation.alternate {
            let (word, _) = alternate_words(alternate);
            args.push(format!("alternate: CollationAlternate.{word}"));
        }
        format!("new Collation({})", args.join(", "))
    }
}
