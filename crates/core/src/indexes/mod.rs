//! Index definitions: the typed spec, the `createIndexes` command and validation of new specs.

mod spec;
#[cfg(test)]
mod tests;
mod validate;

pub use spec::{IndexKey, IndexOptions, IndexSpec, KeyKind};
pub use validate::{IndexProblem, validate_new};
