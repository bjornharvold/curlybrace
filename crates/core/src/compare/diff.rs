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

/// One step of the address of a value inside a document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathSegment {
    Field(String),
    Index(usize),
    /// An element of an unordered array (`[*]`): not addressable, so never a write target.
    AnyElement,
}

/// The display form of a path: `a.b[2].c`, `a[*]`. Ambiguous for field names containing `.` or
/// `[`; the segments are the address.
pub fn render_path(segments: &[PathSegment]) -> String {
    let mut out = String::new();
    for segment in segments {
        match segment {
            PathSegment::Field(name) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(name);
            }
            PathSegment::Index(i) => out.push_str(&format!("[{i}]")),
            PathSegment::AnyElement => out.push_str("[*]"),
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct FieldChange {
    /// `a.b[2].c`; `a[*]` for an element of an unordered array. Always `render_path(&segments)`.
    pub path: String,
    /// The structured address of the change.
    pub segments: Vec<PathSegment>,
    pub kind: ChangeKind,
    pub source: Option<Bson>,
    pub target: Option<Bson>,
}

impl FieldChange {
    /// False when any segment is [`PathSegment::AnyElement`].
    pub fn is_addressable(&self) -> bool {
        !self.segments.contains(&PathSegment::AnyElement)
    }
}

fn bytes(value: &Bson) -> Result<Vec<u8>> {
    let wrapped = Document::from_iter([("v".to_string(), value.clone())]);
    bson::to_vec(&wrapped)
        .map_err(|_| CoreError::InvalidDocument("a document could not be serialized".into()))
}

struct Walker {
    ignore_array_order: bool,
    segments: Vec<PathSegment>,
    out: Vec<FieldChange>,
}

impl Walker {
    fn push(&mut self, kind: ChangeKind, source: Option<&Bson>, target: Option<&Bson>) {
        self.out.push(FieldChange {
            path: render_path(&self.segments),
            segments: self.segments.clone(),
            kind,
            source: source.cloned(),
            target: target.cloned(),
        });
    }

    /// Runs `f` with `segment` appended to the current path, then restores the path.
    fn within<T>(&mut self, segment: PathSegment, f: impl FnOnce(&mut Self) -> T) -> T {
        self.segments.push(segment);
        let result = f(self);
        self.segments.pop();
        result
    }

    fn docs(&mut self, source: &Document, target: &Document) -> Result<()> {
        for (key, sv) in source {
            self.within(PathSegment::Field(key.clone()), |w| match target.get(key) {
                None => {
                    w.push(ChangeKind::Removed, Some(sv), None);
                    Ok(())
                }
                Some(tv) => w.values(sv, tv),
            })?;
        }
        for (key, tv) in target {
            if !source.contains_key(key) {
                self.within(PathSegment::Field(key.clone()), |w| {
                    w.push(ChangeKind::Added, None, Some(tv))
                });
            }
        }
        Ok(())
    }

    fn values(&mut self, source: &Bson, target: &Bson) -> Result<()> {
        match (source, target) {
            (Bson::Document(a), Bson::Document(b)) => self.docs(a, b),
            (Bson::Array(a), Bson::Array(b)) if self.ignore_array_order => self.unordered(a, b),
            (Bson::Array(a), Bson::Array(b)) => self.ordered(a, b),
            _ => {
                if bytes(source)? != bytes(target)? {
                    self.push(ChangeKind::Changed, Some(source), Some(target));
                }
                Ok(())
            }
        }
    }

    fn ordered(&mut self, source: &[Bson], target: &[Bson]) -> Result<()> {
        for i in 0..source.len().max(target.len()) {
            self.within(PathSegment::Index(i), |w| {
                match (source.get(i), target.get(i)) {
                    (Some(s), Some(t)) => w.values(s, t)?,
                    (Some(s), None) => w.push(ChangeKind::Removed, Some(s), None),
                    (None, Some(t)) => w.push(ChangeKind::Added, None, Some(t)),
                    (None, None) => {}
                }
                Ok(())
            })?;
        }
        Ok(())
    }

    /// Both arrays are sorted by canonical bytes (normalization did that), so a merge walk
    /// pairs equal elements and reports the rest, honouring multiplicity.
    fn unordered(&mut self, source: &[Bson], target: &[Bson]) -> Result<()> {
        let keyed = |items: &[Bson]| -> Result<Vec<(Vec<u8>, usize)>> {
            items
                .iter()
                .enumerate()
                .map(|(i, v)| bytes(v).map(|b| (b, i)))
                .collect()
        };
        let (s, t) = (keyed(source)?, keyed(target)?);
        self.within(PathSegment::AnyElement, |w| {
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
                        w.push(ChangeKind::Removed, Some(&source[s[i].1]), None);
                        i += 1;
                    }
                    std::cmp::Ordering::Greater => {
                        w.push(ChangeKind::Added, None, Some(&target[t[j].1]));
                        j += 1;
                    }
                }
            }
        });
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
        segments: Vec::new(),
        out: Vec::new(),
    };
    walker.docs(source, target)?;
    Ok(walker.out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::doc;

    fn d(a: Document, b: Document) -> Vec<FieldChange> {
        diff_normalized(&a, &b, false).unwrap()
    }

    /// Parses simple display paths (`a.b[2].c`, `a[*]`) whose field names contain no `.`/`[`.
    fn parse_path(path: &str) -> Vec<PathSegment> {
        let mut out = Vec::new();
        for part in path.split('.') {
            let (name, mut rest) = part.split_once('[').map_or((part, ""), |(n, r)| (n, r));
            if !name.is_empty() {
                out.push(PathSegment::Field(name.into()));
            }
            while !rest.is_empty() {
                let (idx, tail) = rest.split_once(']').unwrap();
                out.push(match idx {
                    "*" => PathSegment::AnyElement,
                    n => PathSegment::Index(n.parse().unwrap()),
                });
                rest = tail.strip_prefix('[').unwrap_or(tail);
            }
        }
        out
    }

    fn change(
        path: &str,
        kind: ChangeKind,
        source: Option<Bson>,
        target: Option<Bson>,
    ) -> FieldChange {
        FieldChange {
            path: path.into(),
            segments: parse_path(path),
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

    #[test]
    fn a_field_name_with_a_dot_is_one_segment() {
        let s = doc! { "a.b": 1 };
        let t = doc! { "a.b": 2 };
        let changes = diff_normalized(&s, &t, false).unwrap();
        assert_eq!(changes[0].segments, vec![PathSegment::Field("a.b".into())]);
        assert_eq!(changes[0].path, "a.b"); // display form is ambiguous on purpose; segments are the address
    }

    #[test]
    fn nested_and_array_segments_render_to_the_display_path() {
        let s = doc! { "a": { "b": [ { "c": 1 } ] } };
        let t = doc! { "a": { "b": [ { "c": 2 } ] } };
        let c = &diff_normalized(&s, &t, false).unwrap()[0];
        assert_eq!(
            c.segments,
            vec![
                PathSegment::Field("a".into()),
                PathSegment::Field("b".into()),
                PathSegment::Index(0),
                PathSegment::Field("c".into()),
            ]
        );
        assert_eq!(c.path, "a.b[0].c");
        assert!(c.is_addressable());
    }

    #[test]
    fn an_element_of_an_unordered_array_is_not_addressable() {
        let s = doc! { "a": [1, 2] };
        let t = doc! { "a": [1, 3] };
        let changes = diff_normalized(&s, &t, true).unwrap();
        assert!(changes.iter().all(|c| !c.is_addressable()));
        assert!(changes.iter().all(|c| c.path == "a[*]"));
    }
}
