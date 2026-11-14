//! Request, options and error types of the code generator.

use bson::Document;

use crate::codegen::keywords::is_reserved_name;
use crate::store::{Namespace, Query};

/// A target language for generated driver code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Language {
    Mongosh,
    Node,
    Python,
    Java,
    CSharp,
    Go,
    Php,
    Ruby,
    Rust,
}

impl Language {
    pub const ALL: [Language; 9] = [
        Language::Mongosh,
        Language::Node,
        Language::Python,
        Language::Java,
        Language::CSharp,
        Language::Go,
        Language::Php,
        Language::Ruby,
        Language::Rust,
    ];

    /// Stable lowercase key for i18n lookups, fixture folders and tests.
    pub fn key(self) -> &'static str {
        match self {
            Language::Mongosh => "mongosh",
            Language::Node => "node",
            Language::Python => "python",
            Language::Java => "java",
            Language::CSharp => "csharp",
            Language::Go => "go",
            Language::Php => "php",
            Language::Ruby => "ruby",
            Language::Rust => "rust",
        }
    }
}

/// What to generate code for. Aggregate stages are exactly what a run sends.
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::large_enum_variant)] // short-lived value; boxing would burden every caller
pub enum CodeRequest {
    Find {
        ns: Namespace,
        query: Query,
    },
    Aggregate {
        ns: Namespace,
        stages: Vec<Document>,
    },
}

/// Generation options. Names are validated by [`CodeOptions::validated`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeOptions {
    /// Include imports, the client and the database/collection handles.
    pub setup: bool,
    pub client_var: String,
    pub db_var: String,
    pub collection_var: String,
    /// Export a trailing `$out` / `$merge` stage; when off it is dropped.
    pub write_stage: bool,
}

impl Default for CodeOptions {
    fn default() -> Self {
        Self {
            setup: true,
            client_var: "client".into(),
            db_var: "db".into(),
            collection_var: "collection".into(),
            write_stage: true,
        }
    }
}

const MAX_NAME_LEN: usize = 41;

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let first_ok = matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_');
    first_ok
        && name.len() <= MAX_NAME_LEN
        && name != "_"
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn check_name(which: &'static str, name: &str) -> Result<(), CodegenError> {
    if is_identifier(name) && !is_reserved_name(name) {
        Ok(())
    } else {
        Err(CodegenError::InvalidName(which))
    }
}

impl CodeOptions {
    /// Checks that every variable name is identifier-safe in all nine
    /// languages, not a keyword of any, not a name the generator itself uses,
    /// and pairwise distinct, so an option can never inject code.
    pub fn validated(self) -> Result<Self, CodegenError> {
        check_name("client", &self.client_var)?;
        check_name("db", &self.db_var)?;
        check_name("collection", &self.collection_var)?;
        // mongosh's own global `db` would be shadowed by the collection handle.
        if self.collection_var == "db" {
            return Err(CodegenError::InvalidName("collection"));
        }
        let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
        if same(&self.client_var, &self.db_var) {
            return Err(CodegenError::InvalidName("db"));
        }
        if same(&self.client_var, &self.collection_var) || same(&self.db_var, &self.collection_var)
        {
            return Err(CodegenError::InvalidName("collection"));
        }
        Ok(self)
    }
}

/// Why code could not be generated. Messages never contain request content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodegenError {
    /// Which option (`client`, `db` or `collection`) has an unusable name.
    InvalidName(&'static str),
    /// No stage to export (empty pipeline, or only a dropped write stage).
    EmptyPipeline,
    /// A BSON type (by name) the language cannot express.
    UnsupportedValue(&'static str),
}

impl CodegenError {
    /// Stable variant name for logs and tests.
    pub fn error_variant(&self) -> &'static str {
        match self {
            CodegenError::InvalidName(_) => "InvalidName",
            CodegenError::EmptyPipeline => "EmptyPipeline",
            CodegenError::UnsupportedValue(_) => "UnsupportedValue",
        }
    }
}

impl std::fmt::Display for CodegenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodegenError::InvalidName(which) => write!(f, "invalid {which} variable name"),
            CodegenError::EmptyPipeline => write!(f, "the pipeline has no stage to export"),
            CodegenError::UnsupportedValue(kind) => {
                write!(f, "a {kind} value cannot be expressed in this language")
            }
        }
    }
}

impl std::error::Error for CodegenError {}
