//! The field-level difference between two normalized documents, for the detail view and (later)
//! field-level apply. Both inputs must already be normalized with the same rules. Equality is
//! byte equality of the BSON, the same notion the fingerprint uses, so `NaN` equals `NaN` and
//! `Int32 1` differs from `Int64 1`.

use bson::{Bson, Document};

use crate::error::{CoreError, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    /// Present only in the target.
    Added,
    /// Present only in the source.
    Removed,
    /// Present in both, with different values.
    Changed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FieldChange {
    /// `a.b[2].c`; `a[*]` for an element of an unordered array.
    pub path: String,
    pub kind: ChangeKind,
    pub source: Option<Bson>,
    pub target: Option<Bson>,
}

fn bytes(value: &Bson) -> Result<Vec<u8>> {
    let wrapped = Document::from_iter([("v".to_string(), value.clone())]);
    bson::to_vec(&wrapped)
        .map_err(|_| CoreError::InvalidDocument("a document could not be serialized".into()))
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

struct Walker {
    ignore_array_order: bool,
    out: Vec<FieldChange>,
}

impl Walker {
    fn push(
        &mut self,
        path: String,
        kind: ChangeKind,
        source: Option<&Bson>,
        target: Option<&Bson>,
    ) {
        self.out.push(FieldChange {
            path,
            kind,
            source: source.cloned(),
            target: target.cloned(),
        });
    }

    fn docs(&mut self, source: &Document, target: &Document, path: &str) -> Result<()> {
        for (key, sv) in source {
            let p = join(path, key);
            match target.get(key) {
                None => self.push(p, ChangeKind::Removed, Some(sv), None),
                Some(tv) => self.values(sv, tv, &p)?,
            }
        }
        for (key, tv) in target {
            if !source.contains_key(key) {
                self.push(join(path, key), ChangeKind::Added, None, Some(tv));
            }
        }
        Ok(())
    }

    fn values(&mut self, source: &Bson, target: &Bson, path: &str) -> Result<()> {
        match (source, target) {
            (Bson::Document(a), Bson::Document(b)) => self.docs(a, b, path),
            (Bson::Array(a), Bson::Array(b)) if self.ignore_array_order => {
                self.unordered(a, b, path)
            }
            (Bson::Array(a), Bson::Array(b)) => self.ordered(a, b, path),
            _ => {
                if bytes(source)? != bytes(target)? {
                    self.push(
                        path.to_string(),
                        ChangeKind::Changed,
                        Some(source),
                        Some(target),
                    );
                }
                Ok(())
            }
        }
    }

    fn ordered(&mut self, source: &[Bson], target: &[Bson], path: &str) -> Result<()> {
        for i in 0..source.len().max(target.len()) {
            let p = format!("{path}[{i}]");
            match (source.get(i), target.get(i)) {
                (Some(s), Some(t)) => self.values(s, t, &p)?,
                (Some(s), None) => self.push(p, ChangeKind::Removed, Some(s), None),
                (None, Some(t)) => self.push(p, ChangeKind::Added, None, Some(t)),
                (None, None) => {}
            }
        }
        Ok(())
    }

    /// Both arrays are sorted by canonical bytes (normalization did that), so a merge walk
    /// pairs equal elements and reports the rest, honouring multiplicity.
    fn unordered(&mut self, source: &[Bson], target: &[Bson], path: &str) -> Result<()> {
        let keyed = |items: &[Bson]| -> Result<Vec<(Vec<u8>, usize)>> {
            items
                .iter()
                .enumerate()
                .map(|(i, v)| bytes(v).map(|b| (b, i)))
                .collect()
        };
        let (s, t) = (keyed(source)?, keyed(target)?);
        let p = format!("{path}[*]");
        let (mut i, mut j) = (0, 0);
        while i < s.len() || j < t.len() {
            let order = match (s.get(i), t.get(j)) {
                (Some(a), Some(b)) => a.0.cmp(&b.0),
                (Some(_), None) => std::cmp::Ordering::Less,
                _ => std::cmp::Ordering::Greater,
            };
            match order {
                std::cmp::Ordering::Equal => {
                    i += 1;
                    j += 1;
                }
                std::cmp::Ordering::Less => {
                    self.push(p.clone(), ChangeKind::Removed, Some(&source[s[i].1]), None);
                    i += 1;
                }
                std::cmp::Ordering::Greater => {
                    self.push(p.clone(), ChangeKind::Added, None, Some(&target[t[j].1]));
                    j += 1;
                }
            }
        }
        Ok(())
    }
}

/// The changes that turn `source` into `target`, in document order.
pub fn diff_normalized(
    source: &Document,
    target: &Document,
    ignore_array_order: bool,
) -> Result<Vec<FieldChange>> {
    let mut walker = Walker {
        ignore_array_order,
        out: Vec::new(),
    };
    walker.docs(source, target, "")?;
    Ok(walker.out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::doc;

    fn d(a: Document, b: Document) -> Vec<FieldChange> {
        diff_normalized(&a, &b, false).unwrap()
    }

    fn change(
        path: &str,
        kind: ChangeKind,
        source: Option<Bson>,
        target: Option<Bson>,
    ) -> FieldChange {
        FieldChange {
            path: path.into(),
            kind,
            source,
            target,
        }
    }

    #[test]
    fn identical_documents_have_no_changes() {
        assert!(
            d(
                doc! { "a": 1, "b": { "c": [1, 2] } },
                doc! { "a": 1, "b": { "c": [1, 2] } }
            )
            .is_empty()
        );
    }

    #[test]
    fn a_changed_scalar_reports_its_path() {
        assert_eq!(
            d(doc! { "a": { "b": 1 } }, doc! { "a": { "b": 2 } }),
            vec![change(
                "a.b",
                ChangeKind::Changed,
                Some(Bson::Int32(1)),
                Some(Bson::Int32(2))
            )]
        );
    }

    #[test]
    fn fields_present_on_one_side_are_removed_or_added() {
        assert_eq!(
            d(doc! { "x": 1, "keep": 0 }, doc! { "keep": 0, "y": 2 }),
            vec![
                change("x", ChangeKind::Removed, Some(Bson::Int32(1)), None),
                change("y", ChangeKind::Added, None, Some(Bson::Int32(2))),
            ]
        );
    }

    #[test]
    fn a_type_only_difference_is_a_change() {
        assert_eq!(
            d(doc! { "n": 1_i32 }, doc! { "n": 1_i64 }),
            vec![change(
                "n",
                ChangeKind::Changed,
                Some(Bson::Int32(1)),
                Some(Bson::Int64(1))
            )]
        );
    }

    #[test]
    fn a_value_that_changes_kind_is_one_change_at_that_path() {
        assert_eq!(
            d(doc! { "a": { "b": 1 } }, doc! { "a": [1] }),
            vec![change(
                "a",
                ChangeKind::Changed,
                Some(Bson::Document(doc! { "b": 1 })),
                Some(Bson::Array(vec![Bson::Int32(1)]))
            )]
        );
    }

    #[test]
    fn ordered_arrays_diff_by_index_and_report_length_differences() {
        assert_eq!(
            d(doc! { "a": [1, 2, 3] }, doc! { "a": [1, 9] }),
            vec![
                change(
                    "a[1]",
                    ChangeKind::Changed,
                    Some(Bson::Int32(2)),
                    Some(Bson::Int32(9))
                ),
                change("a[2]", ChangeKind::Removed, Some(Bson::Int32(3)), None),
            ]
        );
        assert_eq!(
            d(doc! { "a": [1] }, doc! { "a": [1, 5] }),
            vec![change(
                "a[1]",
                ChangeKind::Added,
                None,
                Some(Bson::Int32(5))
            )]
        );
    }

    #[test]
    fn documents_inside_arrays_diff_recursively() {
        assert_eq!(
            d(doc! { "a": [{ "p": 1 }] }, doc! { "a": [{ "p": 2 }] }),
            vec![change(
                "a[0].p",
                ChangeKind::Changed,
                Some(Bson::Int32(1)),
                Some(Bson::Int32(2))
            )]
        );
    }

    #[test]
    fn unordered_arrays_report_elements_present_on_only_one_side() {
        // Inputs are normalized, so unordered arrays arrive sorted.
        let out =
            diff_normalized(&doc! { "a": [1, 2, 3] }, &doc! { "a": [2, 3, 4] }, true).unwrap();
        assert_eq!(
            out,
            vec![
                change("a[*]", ChangeKind::Removed, Some(Bson::Int32(1)), None),
                change("a[*]", ChangeKind::Added, None, Some(Bson::Int32(4))),
            ]
        );
    }

    #[test]
    fn unordered_arrays_respect_multiplicity() {
        let out = diff_normalized(&doc! { "a": [1, 1, 2] }, &doc! { "a": [1, 2] }, true).unwrap();
        assert_eq!(
            out,
            vec![change(
                "a[*]",
                ChangeKind::Removed,
                Some(Bson::Int32(1)),
                None
            )]
        );
    }

    #[test]
    fn nan_equals_nan_like_the_fingerprint_does() {
        assert!(d(doc! { "x": f64::NAN }, doc! { "x": f64::NAN }).is_empty());
    }
}
