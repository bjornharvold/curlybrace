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

/// The optional imports of a snippet.
#[derive(Clone, Copy, Default)]
struct Imports {
    /// The read preference is set: the code names `readpref`.
    readpref: bool,
    /// The time limit is set: the code builds a context deadline.
    time: bool,
}

impl Go {
    fn frame(
        ns: &Namespace,
        options: &CodeOptions,
        syn: &GoSyntax,
        body: &str,
        imports: Imports,
    ) -> String {
        if !options.setup {
            return body.to_string();
        }
        let math = if syn.uses_math() { "\t\"math\"\n" } else { "" };
        let time = if imports.time { "\t\"time\"\n" } else { "" };
        let readpref = if imports.readpref {
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
            "// Uses the v2 driver: go.mongodb.org/mongo-driver/v2\npackage main\n\nimport (\n\t\"context\"\n\t\"fmt\"\n\t\"log\"\n{math}{time}\n\t\"go.mongodb.org/mongo-driver/v2/bson\"\n\t\"go.mongodb.org/mongo-driver/v2/mongo\"\n\t\"go.mongodb.org/mongo-driver/v2/mongo/options\"\n{readpref})\n\nfunc main() {{\n\t{client}, err := mongo.Connect(options.Client().ApplyURI(\"<your connection string>\"))\n\tif err != nil {{\n\t\tlog.Fatal(err)\n\t}}\n\tdefer func() {{\n\t\tif err := {client}.Disconnect(context.TODO()); err != nil {{\n\t\t\tlog.Fatal(err)\n\t\t}}\n\t}}()\n\n\t{db} := {client}.Database({})\n\t{coll} := {db}.Collection({})\n\n{}\n}}\n",
            quote(&ns.db),
            quote(&ns.collection),
            indent_lines(body.trim_end(), "\t", 1),
        )
    }
}

/// The names the time limit's context deadline declares, and the package it imports.
const DEADLINE_NAMES: [&str; 3] = ["time", "ctx", "cancel"];

/// A variable of one of the deadline's names would shadow it: refused, only when a time limit is
/// set (otherwise those names were always fine).
fn check_deadline_names(options: &CodeOptions) -> Result<(), CodegenError> {
    for (which, name) in [
        ("client", &options.client_var),
        ("db", &options.db_var),
        ("collection", &options.collection_var),
    ] {
        if DEADLINE_NAMES.contains(&name.as_str()) {
            return Err(CodegenError::InvalidName(which));
        }
    }
    Ok(())
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

/// Reads the cursor into `results` under the context `ctx`.
fn read_results(ctx: &str) -> String {
    format!(
        "if err != nil {{\n\tlog.Fatal(err)\n}}\nvar results []bson.M\nif err := cursor.All({ctx}, &results); err != nil {{\n\tlog.Fatal(err)\n}}\nfmt.Println(results)\n"
    )
}

/// What the code says where a time limit is a context deadline (v2 has no `maxTimeMS` option on
/// `Find`); generated code is not UI text, so it is English.
const DEADLINE_NOTE: &str =
    "// The v2 driver has no maxTimeMS option on Find, so the time limit is a context deadline.";

impl Renderer for Go {
    fn find(
        &self,
        ns: &Namespace,
        query: &Query,
        agg: &CodeAggOptions,
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
        setters.extend(Self::option_setters(&syn, agg, false)?);
        let coll = Self::handle(agg, &options.collection_var);
        let ctx = if agg.max_time_ms.is_some() {
            check_deadline_names(options)?;
            "ctx"
        } else {
            "context.TODO()"
        };
        body.push('\n');
        if let Some(ms) = agg.max_time_ms {
            body.push_str(&format!(
                "{DEADLINE_NOTE}\nctx, cancel := context.WithTimeout(context.TODO(), {ms}*time.Millisecond)\ndefer cancel()\n\n"
            ));
        }
        if setters.is_empty() {
            body.push_str(&format!("cursor, err := {coll}.Find({ctx}, filter)\n"));
        } else {
            let chain: String = setters.iter().map(|s| format!(".\n\t{s}")).collect();
            body.push_str(&format!(
                "opts := options.Find(){chain}\n\ncursor, err := {coll}.Find({ctx}, filter, opts)\n"
            ));
        }
        body.push_str(&read_results(ctx));
        let imports = Imports {
            readpref: agg.read_preference.is_some(),
            time: agg.max_time_ms.is_some(),
        };
        Ok(Self::frame(ns, options, &syn, &body, imports))
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
        let handle = Self::handle(agg, &options.collection_var);
        let setters = Self::option_setters(&syn, agg, true)?;
        let (declaration, argument) = if setters.is_empty() {
            (String::new(), "")
        } else {
            let chain: String = setters.iter().map(|s| format!(".\n\t{s}")).collect();
            (format!("opts := options.Aggregate(){chain}\n\n"), ", opts")
        };
        let body = format!(
            "pipeline := {pipeline}\n\n{declaration}cursor, err := {handle}.Aggregate(context.TODO(), pipeline{argument})\n{}",
            read_results("context.TODO()")
        );
        let body = agg.with_note("//", body);
        let imports = Imports {
            readpref: agg.read_preference.is_some(),
            time: false,
        };
        Ok(Self::frame(ns, options, &syn, &body, imports))
    }
}

impl Go {
    /// The collection, or a handle derived from it that reads where the read preference says.
    fn handle(agg: &CodeAggOptions, coll: &str) -> String {
        match agg.read_preference {
            Some(mode) => format!(
                "{coll}.Clone(options.Collection().SetReadPreference(readpref.{}()))",
                mode_words(mode).pascal
            ),
            None => coll.to_string(),
        }
    }

    /// The setters of `options.Aggregate()` or `options.Find()`, in the order collation, hint,
    /// allowDiskUse, maxTimeMS, comment. v2 has no `SetMaxTime`: the aggregate's time limit goes
    /// through `SetCustom` (`custom_time`); the find has no custom options and the caller turns
    /// its time limit into a context deadline.
    fn option_setters(
        syn: &GoSyntax,
        agg: &CodeAggOptions,
        custom_time: bool,
    ) -> Result<Vec<String>, CodegenError> {
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
        if let (true, Some(ms)) = (custom_time, agg.max_time_ms) {
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
