//! Value-free query shapes: a filter, sort and projection reduced to field names and operators
//! (every literal becomes `"?"`), so equal-shaped queries group together and no data of the
//! author's is ever stored. A shape has a stable canonical JSON form and a SHA-256 hash of it.

mod normalize;
#[cfg(test)]
mod normalize_tests;
mod suggest;
#[cfg(test)]
mod suggest_tests;

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::error::CoreError;

pub use normalize::{from_profile_command, from_query};
pub use suggest::suggest_for_shape;

pub(crate) const PLACEHOLDER: &str = "?";
pub(crate) const FILTER_KEY: &str = "filter";
pub(crate) const SORT_KEY: &str = "sort";
pub(crate) const PROJECTION_KEY: &str = "projection";
pub(crate) const INCLUDE_KEY: &str = "include";
pub(crate) const EXCLUDE_KEY: &str = "exclude";

/// The canonical, literal-free form of a query. Build one with [`from_query`],
/// [`from_profile_command`] or [`Shape::from_json`].
#[derive(Debug, Clone)]
pub struct Shape {
    canonical: Map<String, Value>,
}

/// Equal when the canonical JSON is equal; unlike a map comparison this sees the sort's key order.
impl PartialEq for Shape {
    fn eq(&self, other: &Self) -> bool {
        self.json() == other.json()
    }
}

impl Shape {
    pub(crate) fn new(canonical: Map<String, Value>) -> Self {
        Self { canonical }
    }

    pub(crate) fn filter(&self) -> Option<&Map<String, Value>> {
        self.canonical.get(FILTER_KEY).and_then(Value::as_object)
    }

    pub(crate) fn sort(&self) -> Option<&Map<String, Value>> {
        self.canonical.get(SORT_KEY).and_then(Value::as_object)
    }

    /// Stable canonical JSON; holds field names, operators and `"?"` placeholders only.
    pub fn json(&self) -> String {
        Value::Object(self.canonical.clone()).to_string()
    }

    /// Lowercase hex SHA-256 (64 characters) of [`Shape::json`].
    pub fn hash(&self) -> String {
        Sha256::digest(self.json().as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    /// Parses what [`Shape::json`] produced. Anything else, including JSON that carries a
    /// literal value, is rejected.
    pub fn from_json(json: &str) -> Result<Shape, CoreError> {
        let value: Value = serde_json::from_str(json).map_err(|e| invalid(&e.to_string()))?;
        let Value::Object(canonical) = value else {
            return Err(invalid("not an object"));
        };
        normalize::validate(&canonical).map_err(|why| invalid(&why))?;
        Ok(Shape::new(canonical))
    }
}

fn invalid(why: &str) -> CoreError {
    CoreError::Database(format!("not a query shape: {why}"))
}
