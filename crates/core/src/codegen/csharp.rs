//! .NET driver renderer (C# top-level statements).

use bson::Document;

use crate::codegen::Renderer;
use crate::codegen::csharp_syntax::{CsharpSyntax, quote};
use crate::codegen::model::{CodeOptions, CodegenError};
use crate::codegen::walk::{render_document, render_documents};
use crate::store::{Namespace, Query};

pub(crate) struct Csharp;

impl Csharp {
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
        options: &CodeOptions,
    ) -> Result<String, CodegenError> {
        let syn = CsharpSyntax;
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
        Ok(Self::frame(ns, options, &body))
    }

    fn aggregate(
        &self,
        ns: &Namespace,
        stages: &[Document],
        options: &CodeOptions,
    ) -> Result<String, CodegenError> {
        let pipeline = render_documents(&CsharpSyntax, stages, 0)?;
        let body = format!(
            "var pipeline = {pipeline};\n\nvar results = {}.Aggregate<BsonDocument>(pipeline).ToList();\nresults.ForEach(Console.WriteLine);\n",
            options.collection_var
        );
        Ok(Self::frame(ns, options, &body))
    }
}
