//! Go driver (v2) renderer.

use bson::Document;

use crate::codegen::Renderer;
use crate::codegen::agg_options::{CodeAggOptions, mode_words};
use crate::codegen::escape::indent_lines;
use crate::codegen::go_syntax::{GoSyntax, quote};
use crate::codegen::model::{CodeOptions, CodegenError};
use crate::codegen::range::i64_arg;
use crate::codegen::walk::{Syntax, render, render_document, render_documents};
use crate::pipeline_options::CollationOption;
use crate::store::{Namespace, Query};

pub(crate) struct Go;

impl Go {
    fn frame(
        ns: &Namespace,
        options: &CodeOptions,
        syn: &GoSyntax,
        body: &str,
        readpref: bool,
    ) -> String {
        if !options.setup {
            return body.to_string();
        }
        let math = if syn.uses_math() { "\t\"math\"\n" } else { "" };
        let readpref = if readpref {
            "\t\"go.mongodb.org/mongo-driver/v2/mongo/readpref\"\n"
        } else {
            ""
        };
        let (client, db, coll) = (
            &options.client_var,
            &options.db_var,
            &options.collection_var,
        );
        format!(
            "// Uses the v2 driver: go.mongodb.org/mongo-driver/v2\npackage main\n\nimport (\n\t\"context\"\n\t\"fmt\"\n\t\"log\"\n{math}\n\t\"go.mongodb.org/mongo-driver/v2/bson\"\n\t\"go.mongodb.org/mongo-driver/v2/mongo\"\n\t\"go.mongodb.org/mongo-driver/v2/mongo/options\"\n{readpref})\n\nfunc main() {{\n\t{client}, err := mongo.Connect(options.Client().ApplyURI(\"<your connection string>\"))\n\tif err != nil {{\n\t\tlog.Fatal(err)\n\t}}\n\tdefer func() {{\n\t\tif err := {client}.Disconnect(context.TODO()); err != nil {{\n\t\t\tlog.Fatal(err)\n\t\t}}\n\t}}()\n\n\t{db} := {client}.Database({})\n\t{coll} := {db}.Collection({})\n\n{}\n}}\n",
            quote(&ns.db),
            quote(&ns.collection),
            indent_lines(body.trim_end(), "\t", 1),
        )
    }
}

/// Package names the snippet imports; a variable of that name would shadow one.
const PACKAGE_NAMES: [&str; 7] = ["bson", "context", "fmt", "log", "math", "mongo", "readpref"];

fn check_names(options: &CodeOptions) -> Result<(), CodegenError> {
    let clash = |name: &str| PACKAGE_NAMES.contains(&name) || name == "options";
    for (which, name) in [
        ("client", &options.client_var),
        ("db", &options.db_var),
        ("collection", &options.collection_var),
    ] {
        if clash(name) {
            return Err(CodegenError::InvalidName(which));
        }
    }
    Ok(())
}

const READ_RESULTS: &str = "if err != nil {\n\tlog.Fatal(err)\n}\nvar results []bson.M\nif err := cursor.All(context.TODO(), &results); err != nil {\n\tlog.Fatal(err)\n}\nfmt.Println(results)\n";

impl Renderer for Go {
    fn find(
        &self,
        ns: &Namespace,
        query: &Query,
        _agg: &CodeAggOptions,
        options: &CodeOptions,
    ) -> Result<String, CodegenError> {
        check_names(options)?;
        let syn = GoSyntax::new();
        let mut body = format!("filter := {}\n", render_document(&syn, &query.filter, 0)?);
        let mut setters: Vec<String> = Vec::new();
        if let Some(projection) = &query.projection {
            body.push_str(&format!(
                "projection := {}\n",
                render_document(&syn, projection, 0)?
            ));
            setters.push("SetProjection(projection)".into());
        }
        if let Some(sort) = &query.sort {
            body.push_str(&format!("sort := {}\n", render_document(&syn, sort, 0)?));
            setters.push("SetSort(sort)".into());
        }
        if query.skip > 0 {
            setters.push(format!("SetSkip({})", i64_arg("skip", query.skip)?));
        }
        if query.limit > 0 {
            setters.push(format!("SetLimit({})", i64_arg("limit", query.limit)?));
        }
        let coll = &options.collection_var;
        if setters.is_empty() {
            body.push_str(&format!(
                "\ncursor, err := {coll}.Find(context.TODO(), filter)\n"
            ));
        } else {
            let chain: String = setters.iter().map(|s| format!(".\n\t{s}")).collect();
            body.push_str(&format!(
                "\nopts := options.Find(){chain}\n\ncursor, err := {coll}.Find(context.TODO(), filter, opts)\n"
            ));
        }
        body.push_str(READ_RESULTS);
        Ok(Self::frame(ns, options, &syn, &body, false))
    }

    fn aggregate(
        &self,
        ns: &Namespace,
        stages: &[Document],
        agg: &CodeAggOptions,
        options: &CodeOptions,
    ) -> Result<String, CodegenError> {
        check_names(options)?;
        let syn = GoSyntax::new();
        let pipeline = render_documents(&syn, stages, 0)?;
        let handle = match agg.read_preference {
            Some(mode) => format!(
                "{}.Clone(options.Collection().SetReadPreference(readpref.{}()))",
                options.collection_var,
                mode_words(mode).pascal
            ),
            None => options.collection_var.clone(),
        };
        let setters = Self::option_setters(&syn, agg)?;
        let (declaration, argument) = if setters.is_empty() {
            (String::new(), "")
        } else {
            let chain: String = setters.iter().map(|s| format!(".\n\t{s}")).collect();
            (format!("opts := options.Aggregate(){chain}\n\n"), ", opts")
        };
        let body = format!(
            "pipeline := {pipeline}\n\n{declaration}cursor, err := {handle}.Aggregate(context.TODO(), pipeline{argument})\n{READ_RESULTS}"
        );
        let body = agg.with_note("//", body);
        Ok(Self::frame(
            ns,
            options,
            &syn,
            &body,
            agg.read_preference.is_some(),
        ))
    }
}

impl Go {
    /// The setters of `options.Aggregate()`, in the order collation, hint, allowDiskUse,
    /// maxTimeMS, comment. v2 has no `SetMaxTime`, so the time limit goes through `SetCustom`.
    fn option_setters(syn: &GoSyntax, agg: &CodeAggOptions) -> Result<Vec<String>, CodegenError> {
        let mut setters = Vec::new();
        if let Some(collation) = &agg.collation {
            setters.push(format!("SetCollation({})", Self::collation(collation)));
        }
        if let Some(hint) = &agg.hint {
            setters.push(format!("SetHint({})", render(syn, &hint.to_bson(), 1)?));
        }
        if let Some(on) = agg.allow_disk_use {
            setters.push(format!("SetAllowDiskUse({on})"));
        }
        if let Some(ms) = agg.max_time_ms {
            let value = syn.int32(ms);
            setters.push(format!("SetCustom(bson.M{{\"maxTimeMS\": {value}}})"));
        }
        if let Some(comment) = &agg.comment {
            setters.push(format!("SetComment({})", quote(comment)));
        }
        Ok(setters)
    }

    /// `&options.Collation{..}`: a bool set to `false` is the server's default and is left out.
    fn collation(collation: &CollationOption) -> String {
        let mut fields = vec![format!("Locale: {}", quote(&collation.locale))];
        if collation.case_level == Some(true) {
            fields.push("CaseLevel: true".into());
        }
        if let Some(case_first) = collation.case_first {
            fields.push(format!("CaseFirst: {}", quote(case_first.word())));
        }
        if let Some(strength) = collation.strength {
            fields.push(format!("Strength: {}", u8::from(strength)));
        }
        if collation.numeric_ordering == Some(true) {
            fields.push("NumericOrdering: true".into());
        }
        if let Some(alternate) = collation.alternate {
            fields.push(format!("Alternate: {}", quote(alternate.word())));
        }
        format!("&options.Collation{{{}}}", fields.join(", "))
    }
}
