//! A 128-bit fingerprint of a normalized document: the first 16 bytes of SHA-256 over its BSON
//! bytes. Apply (slice 2) recomputes it to detect a document that changed since the compare.

use std::fmt;

use bson::Document;
use sha2::{Digest, Sha256};

use super::normalize::normalize;
use super::rules::CompareRules;
#[cfg(test)]
use super::rules::FieldScope;
use crate::error::{CoreError, Result};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fingerprint(pub [u8; 16]);

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fingerprint({})", hex(&self.0))
    }
}

/// Fingerprint of an already normalized document.
pub fn fingerprint(normalized: &Document) -> Result<Fingerprint> {
    let bytes = bson::to_vec(normalized)
        .map_err(|_| CoreError::InvalidDocument("a document could not be serialized".into()))?;
    let digest = Sha256::digest(&bytes);
    let mut out = [0u8; 16];
    out.copy_from_slice(&digest[..16]);
    Ok(Fingerprint(out))
}

/// Normalizes under `rules`, then fingerprints.
pub fn fingerprint_of(doc: &Document, rules: &CompareRules) -> Result<Fingerprint> {
    fingerprint(&normalize(doc, rules)?)
}

#[cfg(test)]
const EXPECTED_ID_1: &str = "2fe93d7561368009c932ffef6a24dbbf";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compare::key::KeySpec;
    use bson::doc;

    fn rules() -> CompareRules {
        CompareRules::for_key(&KeySpec::id())
    }

    #[test]
    fn equal_documents_share_a_fingerprint_and_a_change_alters_it() {
        let a = doc! { "_id": 1, "a": 1, "b": "x" };
        assert_eq!(
            fingerprint_of(&a, &rules()).unwrap(),
            fingerprint_of(&a.clone(), &rules()).unwrap()
        );
        let changed = doc! { "_id": 1, "a": 2, "b": "x" };
        assert_ne!(
            fingerprint_of(&a, &rules()).unwrap(),
            fingerprint_of(&changed, &rules()).unwrap()
        );
    }

    #[test]
    fn types_are_distinguished_under_strict_rules() {
        let int = doc! { "_id": 1, "n": 1_i32 };
        let long = doc! { "_id": 1, "n": 1_i64 };
        assert_ne!(
            fingerprint_of(&int, &rules()).unwrap(),
            fingerprint_of(&long, &rules()).unwrap()
        );
        let mut loose = rules();
        loose.numbers_equal_across_types = true;
        assert_eq!(
            fingerprint_of(&int, &loose).unwrap(),
            fingerprint_of(&long, &loose).unwrap()
        );
    }

    #[test]
    fn fields_outside_the_scope_never_affect_the_fingerprint() {
        let mut r = rules();
        r.field_scope = FieldScope::AllExcept(vec!["updatedAt".into()]);
        let a = doc! { "_id": 1, "v": 1, "updatedAt": 1 };
        let b = doc! { "_id": 1, "v": 1, "updatedAt": 999 };
        assert_eq!(
            fingerprint_of(&a, &r).unwrap(),
            fingerprint_of(&b, &r).unwrap()
        );
    }

    #[test]
    fn the_fingerprint_is_sixteen_bytes_and_prints_as_hex() {
        let f = fingerprint_of(&doc! { "_id": 1 }, &rules()).unwrap();
        assert_eq!(f.0.len(), 16);
        assert_eq!(format!("{f:?}").len(), "Fingerprint()".len() + 32);
    }

    #[test]
    fn a_known_vector_pins_the_hash_so_it_cannot_change_silently() {
        // SHA-256 of the BSON bytes of {"_id": 1}, first 16 bytes. If this changes, every saved
        // comparison's fingerprints change meaning: bump deliberately, never by accident.
        let f = fingerprint(&doc! { "_id": 1_i32 }).unwrap();
        assert_eq!(hex(&f.0), EXPECTED_ID_1);
    }
}
