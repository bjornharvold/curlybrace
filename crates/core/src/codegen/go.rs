//! Go driver (v2) renderer.

use bson::Document;

use crate::codegen::Renderer;
use crate::codegen::escape::indent_lines;
use crate::codegen::go_syntax::{GoSyntax, quote};
use crate::codegen::model::{CodeOptions, CodegenError};
use crate::codegen::walk::{render_document, render_documents};
use crate::store::{Namespace, Query};

pub(crate) struct Go;

impl Go {
    fn frame(ns: &Namespace, options: &CodeOptions, syn: &GoSyntax, body: &str) -> String {
        if !options.setup {
            return body.to_string();
        }
        let math = if syn.uses_math() { "\t\"math\"\n" } else { "" };
        let (client, db, coll) = (
            &options.client_var,
            &options.db_var,
            &options.collection_var,
        );
        format!(
            "// Uses the v2 driver: go.mongodb.org/mongo-driver/v2\npackage main\n\nimport (\n\t\"context\"\n\t\"fmt\"\n\t\"log\"\n{math}\n\t\"go.mongodb.org/mongo-driver/v2/bson\"\n\t\"go.mongodb.org/mongo-driver/v2/mongo\"\n\t\"go.mongodb.org/mongo-driver/v2/mongo/options\"\n)\n\nfunc main() {{\n\t{client}, err := mongo.Connect(options.Client().ApplyURI(\"<your connection string>\"))\n\tif err != nil {{\n\t\tlog.Fatal(err)\n\t}}\n\tdefer func() {{\n\t\tif err := {client}.Disconnect(context.TODO()); err != nil {{\n\t\t\tlog.Fatal(err)\n\t\t}}\n\t}}()\n\n\t{db} := {client}.Database({})\n\t{coll} := {db}.Collection({})\n\n{}\n}}\n",
            quote(&ns.db),
            quote(&ns.collection),
            indent_lines(body.trim_end(), "\t", 1),
        )
    }
}

/// Package names the snippet imports; a variable of that name would shadow one.
const PACKAGE_NAMES: [&str; 6] = ["bson", "context", "fmt", "log", "math", "mongo"];

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

/// `SetSkip` / `SetLimit` take `int64`.
fn int_arg(name: &'static str, value: u64) -> Result<i64, CodegenError> {
    i64::try_from(value).map_err(|_| CodegenError::OutOfRange(name))
}

const READ_RESULTS: &str = "if err != nil {\n\tlog.Fatal(err)\n}\nvar results []bson.M\nif err := cursor.All(context.TODO(), &results); err != nil {\n\tlog.Fatal(err)\n}\nfmt.Println(results)\n";

impl Renderer for Go {
    fn find(
        &self,
        ns: &Namespace,
        query: &Query,
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
            setters.push(format!("SetSkip({})", int_arg("skip", query.skip)?));
        }
        if query.limit > 0 {
            setters.push(format!("SetLimit({})", int_arg("limit", query.limit)?));
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
        Ok(Self::frame(ns, options, &syn, &body))
    }

    fn aggregate(
        &self,
        ns: &Namespace,
        stages: &[Document],
        options: &CodeOptions,
    ) -> Result<String, CodegenError> {
        check_names(options)?;
        let syn = GoSyntax::new();
        let pipeline = render_documents(&syn, stages, 0)?;
        let body = format!(
            "pipeline := {pipeline}\n\ncursor, err := {}.Aggregate(context.TODO(), pipeline)\n{READ_RESULTS}",
            options.collection_var
        );
        Ok(Self::frame(ns, options, &syn, &body))
    }
}
