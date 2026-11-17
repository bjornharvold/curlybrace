//! The operation part of the JavaScript snippets (mongosh and Node.js):
//! the variables holding the request and the call that runs it.

use bson::Document;

use crate::codegen::agg_options::{CodeAggOptions, mode_words, plain_entries};
use crate::codegen::js_syntax::JsSyntax;
use crate::codegen::model::CodegenError;
use crate::codegen::range::js_arg;
use crate::codegen::walk::Syntax;
use crate::codegen::walk::{render_document, render_documents};
use crate::store::Query;

/// What differs between the shell and the Node.js driver.
pub(crate) struct JsOps<'a> {
    pub syn: &'a JsSyntax,
    pub collection: &'a str,
    /// Node awaits the cursor; the shell blocks.
    pub awaited: bool,
    /// Node sets the projection with `.project()`; the shell passes it to `find`.
    pub project_method: bool,
    pub print: &'static str,
}

impl JsOps<'_> {
    fn await_prefix(&self) -> &'static str {
        if self.awaited { "await " } else { "" }
    }

    /// The aggregate; with options, an `options` object between the pipeline and the call (the
    /// read preference is an option of the call in both the shell and the Node.js driver).
    pub(crate) fn aggregate(
        &self,
        stages: &[Document],
        agg: &CodeAggOptions,
    ) -> Result<String, CodegenError> {
        let pipeline = render_documents(self.syn, stages, 0)?;
        let coll = self.collection;
        let wait = self.await_prefix();
        let mut entries = plain_entries(self.syn, agg, |name| name.to_string(), 1, None)?;
        if let Some(mode) = agg.read_preference {
            let value = self.syn.string(mode_words(mode).camel);
            entries.push(("readPreference".to_string(), value));
        }
        if entries.is_empty() {
            return Ok(format!(
                "const pipeline = {pipeline};\n\nconst results = {wait}{coll}.aggregate(pipeline).toArray();\n{}\n",
                self.print
            ));
        }
        let options = self.syn.document(&entries, 0);
        Ok(format!(
            "const pipeline = {pipeline};\n\nconst options = {options};\n\nconst results = {wait}{coll}.aggregate(pipeline, options).toArray();\n{}\n",
            self.print
        ))
    }

    /// The find; the options are cursor methods after the sort, skip and limit (in the order
    /// collation, hint, allowDiskUse, maxTimeMS, comment, read preference), except in Node.js,
    /// where the read preference is an option of the `find` call.
    pub(crate) fn find(&self, query: &Query, agg: &CodeAggOptions) -> Result<String, CodegenError> {
        let mut out = format!(
            "const filter = {};\n",
            render_document(self.syn, &query.filter, 0)?
        );
        let mut find_args = "filter".to_string();
        let mut chain: Vec<String> = Vec::new();
        if let Some(projection) = &query.projection {
            out.push_str(&format!(
                "const projection = {};\n",
                render_document(self.syn, projection, 0)?
            ));
            if self.project_method {
                chain.push(".project(projection)".into());
            } else {
                find_args = "filter, projection".to_string();
            }
        }
        if let Some(sort) = &query.sort {
            out.push_str(&format!(
                "const sort = {};\n",
                render_document(self.syn, sort, 0)?
            ));
            chain.push(".sort(sort)".into());
        }
        if query.skip > 0 {
            chain.push(format!(".skip({})", js_arg("skip", query.skip)?));
        }
        if query.limit > 0 {
            chain.push(format!(".limit({})", js_arg("limit", query.limit)?));
        }
        for (name, value) in plain_entries(self.syn, agg, |name| name.to_string(), 1, None)? {
            chain.push(format!(".{name}({value})"));
        }
        if let Some(mode) = agg.read_preference {
            let word = self.syn.string(mode_words(mode).camel);
            if self.awaited {
                let entries = [("readPreference".to_string(), word)];
                find_args = format!("{find_args}, {}", self.syn.document(&entries, 0));
            } else {
                chain.push(format!(".readPref({word})"));
            }
        }
        let coll = self.collection;
        let wait = self.await_prefix();
        let call = if chain.is_empty() {
            format!("{wait}{coll}.find({find_args}).toArray();")
        } else {
            let lines: String = chain.iter().map(|c| format!("\n  {c}")).collect();
            format!("{wait}{coll}\n  .find({find_args}){lines}\n  .toArray();")
        };
        out.push_str(&format!("\nconst results = {call}\n{}\n", self.print));
        Ok(out)
    }
}
