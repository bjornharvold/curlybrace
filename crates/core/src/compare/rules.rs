//! The rules a compare applies before two documents are called equal.

use serde::{Deserialize, Serialize};

use super::key::KeySpec;
use crate::error::{CoreError, Result};

/// Which fields take part in the comparison. Paths are dotted; a path that crosses an array
/// addresses every element of it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldScope {
    /// Compare every field.
    #[default]
    All,
    /// Ignore these paths.
    AllExcept(Vec<String>),
    /// Compare only these paths.
    Only(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompareRules {
    pub field_scope: FieldScope,
    pub ignore_array_order: bool,
    pub numbers_equal_across_types: bool,
    pub ignore_field_order: bool,
    /// Compare `_id` too. When the key is a business key, `_id` differs across environments by
    /// construction, so it is left out unless this is switched on.
    pub include_id: bool,
}

impl CompareRules {
    /// The defaults for a compare keyed on `key`: strict BSON equality, except that the order
    /// of fields inside documents is ignored.
    pub fn for_key(key: &KeySpec) -> Self {
        Self {
            field_scope: FieldScope::All,
            ignore_array_order: false,
            numbers_equal_across_types: false,
            ignore_field_order: true,
            include_id: key.contains_id(),
        }
    }

    /// Checks the user-entered paths.
    pub fn validate(&self) -> Result<()> {
        let paths = match &self.field_scope {
            FieldScope::All => return Ok(()),
            FieldScope::AllExcept(paths) => paths,
            FieldScope::Only(paths) if paths.is_empty() => {
                return Err(CoreError::CompareSetupInvalid);
            }
            FieldScope::Only(paths) => paths,
        };
        for path in paths {
            if path.is_empty() || path.split('.').any(str::is_empty) {
                return Err(CoreError::CompareSetupInvalid);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_the_spec() {
        let r = CompareRules::for_key(&KeySpec::id());
        assert_eq!(r.field_scope, FieldScope::All);
        assert!(!r.ignore_array_order);
        assert!(!r.numbers_equal_across_types);
        assert!(r.ignore_field_order, "field order is ignored by default");
        assert!(r.include_id, "_id is part of the key, so it is compared");
        let business = CompareRules::for_key(&KeySpec::new(vec!["email".into()]).unwrap());
        assert!(
            !business.include_id,
            "_id differs across environments by construction"
        );
    }

    #[test]
    fn validate_rejects_empty_paths_only() {
        let mut r = CompareRules::for_key(&KeySpec::id());
        assert!(r.validate().is_ok());
        r.field_scope = FieldScope::AllExcept(vec!["updatedAt".into(), "meta.synced".into()]);
        assert!(r.validate().is_ok());
        for bad in ["", "a..b", ".a", "a."] {
            r.field_scope = FieldScope::Only(vec![bad.into()]);
            assert_eq!(r.validate(), Err(CoreError::CompareSetupInvalid), "{bad:?}");
        }
        r.field_scope = FieldScope::Only(vec![]);
        assert_eq!(
            r.validate(),
            Err(CoreError::CompareSetupInvalid),
            "an empty 'only' list would compare nothing"
        );
    }

    #[test]
    fn rules_round_trip_through_json_without_secrets() {
        let mut r = CompareRules::for_key(&KeySpec::id());
        r.field_scope = FieldScope::Only(vec!["a.b".into()]);
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<CompareRules>(&json).unwrap(), r);
    }
}
