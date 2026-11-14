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
mod model;
mod mongosh;
mod node;
mod python;
mod python_syntax;
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

fn renderer(language: Language) -> Result<&'static dyn Renderer, CodegenError> {
    match language {
        Language::Mongosh => Ok(&mongosh::Mongosh),
        Language::Node => Ok(&node::Node),
        Language::Python => Ok(&python::Python),
        Language::Java => Ok(&java::Java),
        Language::CSharp => Ok(&csharp::Csharp),
        Language::Go => Ok(&go::Go),
        Language::Php | Language::Ruby | Language::Rust => {
            Err(CodegenError::UnsupportedLanguage(language.key()))
        }
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
    let renderer = renderer(language)?;
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
