//! Turns an aggregation pipeline or a find query into runnable driver code.
//!
//! Pure and offline: nothing here talks to a server. A language is a
//! [`Renderer`] (the frame: imports, client, the operation) over a
//! [`walk::Syntax`] (how each BSON type is spelled); the shared walk, string
//! escaping and calendar helpers live in `walk` and `escape`. Output is
//! deterministic and ends with a newline. Request content is never logged and
//! never appears in an error.

mod csharp;
mod csharp_syntax;
mod escape;
mod go;
mod go_syntax;
mod java;
mod java_syntax;
mod js_ops;
mod js_syntax;
mod keywords;
mod lossy;
mod model;
mod mongosh;
mod node;
mod php;
mod php_syntax;
mod python;
mod python_syntax;
mod ruby;
mod ruby_syntax;
mod rust;
mod rust_syntax;
mod walk;

use bson::Document;

use crate::pipeline::is_write;
use crate::store::{Namespace, Query};

pub use model::{CodeOptions, CodeRequest, CodegenError, Language};

/// One language's snippet frame. Implementations get validated options and a
/// request already cleaned by [`generate`] (never an empty pipeline).
pub(crate) trait Renderer {
    fn find(
        &self,
        ns: &Namespace,
        query: &Query,
        options: &CodeOptions,
    ) -> Result<String, CodegenError>;

    fn aggregate(
        &self,
        ns: &Namespace,
        stages: &[Document],
        options: &CodeOptions,
    ) -> Result<String, CodegenError>;
}

fn renderer(language: Language) -> &'static dyn Renderer {
    match language {
        Language::Mongosh => &mongosh::Mongosh,
        Language::Node => &node::Node,
        Language::Python => &python::Python,
        Language::Java => &java::Java,
        Language::CSharp => &csharp::Csharp,
        Language::Go => &go::Go,
        Language::Php => &php::Php,
        Language::Ruby => &ruby::Ruby,
        Language::Rust => &rust::Rust,
    }
}

/// The stages to export: a trailing `$out` / `$merge` is dropped unless
/// `keep_write` is set. An empty result is an error, never empty code.
fn exported_stages(stages: &[Document], keep_write: bool) -> Result<Vec<Document>, CodegenError> {
    let mut end = stages.len();
    if !keep_write {
        while end > 0 && stage_is_write(&stages[end - 1]) {
            end -= 1;
        }
    }
    if end == 0 {
        return Err(CodegenError::EmptyPipeline);
    }
    Ok(stages[..end].to_vec())
}

fn stage_is_write(stage: &Document) -> bool {
    stage.keys().next().is_some_and(|name| is_write(name))
}

/// Generates the code for `request` in `language`.
pub fn generate(
    language: Language,
    request: &CodeRequest,
    options: &CodeOptions,
) -> Result<String, CodegenError> {
    let options = options.clone().validated()?;
    let renderer = renderer(language);
    match request {
        CodeRequest::Find { ns, query } => renderer.find(ns, query, &options),
        CodeRequest::Aggregate { ns, stages } => {
            let stages = exported_stages(stages, options.write_stage)?;
            renderer.aggregate(ns, &stages, &options)
        }
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_langs;
#[cfg(test)]
mod tests_review;
#[cfg(test)]
mod tests_scripting;
