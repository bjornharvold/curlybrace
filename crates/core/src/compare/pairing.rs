//! Suggests which collections of a source database pair with which of a target database, by name.

use std::collections::BTreeMap;

use crate::store::CollectionKind;

/// One row of the suggested pairing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Suggestion {
    Matched { name: String },
    OnlyInSource { name: String },
    OnlyInTarget { name: String },
}

/// Pairing switches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PairingOptions {
    pub include_system: bool,
}

/// Matches collection names between two databases. Output order: all `Matched` by name, then
/// `OnlyInSource` by name, then `OnlyInTarget` by name (byte order, so it is deterministic).
pub fn suggest(
    source: &[(String, CollectionKind)],
    target: &[(String, CollectionKind)],
    opts: PairingOptions,
) -> Vec<Suggestion> {
    let src = index(source);
    let tgt = index(target);
    let hidden = |name: &str| {
        let view = |m: &BTreeMap<&str, CollectionKind>| m.get(name) == Some(&CollectionKind::View);
        view(&src) || view(&tgt) || (!opts.include_system && name.starts_with("system."))
    };
    let names = |only: &BTreeMap<&str, CollectionKind>,
                 other: &BTreeMap<&str, CollectionKind>,
                 both: bool| {
        only.keys()
            .filter(|n| other.contains_key(*n) == both && !hidden(n))
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
    };
    let matched = names(&src, &tgt, true)
        .into_iter()
        .map(|name| Suggestion::Matched { name });
    let only_src = names(&src, &tgt, false)
        .into_iter()
        .map(|name| Suggestion::OnlyInSource { name });
    let only_tgt = names(&tgt, &src, false)
        .into_iter()
        .map(|name| Suggestion::OnlyInTarget { name });
    matched.chain(only_src).chain(only_tgt).collect()
}

/// Dedupes and sorts a listing by name.
fn index(list: &[(String, CollectionKind)]) -> BTreeMap<&str, CollectionKind> {
    list.iter().map(|(n, k)| (n.as_str(), *k)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(names: &[&str]) -> Vec<(String, CollectionKind)> {
        names
            .iter()
            .map(|n| with(CollectionKind::Collection, n))
            .collect()
    }

    fn with(kind: CollectionKind, name: &str) -> (String, CollectionKind) {
        (name.to_string(), kind)
    }

    fn m(n: &str) -> Suggestion {
        Suggestion::Matched { name: n.into() }
    }
    fn s(n: &str) -> Suggestion {
        Suggestion::OnlyInSource { name: n.into() }
    }
    fn t(n: &str) -> Suggestion {
        Suggestion::OnlyInTarget { name: n.into() }
    }

    #[test]
    fn matches_same_names_and_sorts_groups() {
        let out = suggest(
            &c(&["b", "a", "x"]),
            &c(&["a", "b", "y"]),
            PairingOptions::default(),
        );
        assert_eq!(out, vec![m("a"), m("b"), s("x"), t("y")]);
    }

    #[test]
    fn names_are_case_sensitive() {
        let out = suggest(&c(&["Users"]), &c(&["users"]), PairingOptions::default());
        assert_eq!(out, vec![s("Users"), t("users")]);
    }

    #[test]
    fn view_on_either_side_is_never_offered() {
        let opts = PairingOptions::default();
        let src_view = [with(CollectionKind::View, "v")];
        assert!(suggest(&src_view, &c(&["v"]), opts).is_empty());
        let tgt_view = [with(CollectionKind::View, "v")];
        assert!(suggest(&c(&["v"]), &tgt_view, opts).is_empty());
    }

    #[test]
    fn timeseries_counts_as_a_collection() {
        let ts = [with(CollectionKind::Timeseries, "t")];
        let out = suggest(&ts, &ts, PairingOptions::default());
        assert_eq!(out, vec![m("t")]);
    }

    #[test]
    fn system_collections_hidden_unless_enabled() {
        let names = c(&["system.profile"]);
        assert!(suggest(&names, &names, PairingOptions::default()).is_empty());
        let on = PairingOptions {
            include_system: true,
        };
        assert_eq!(suggest(&names, &names, on), vec![m("system.profile")]);
    }

    #[test]
    fn duplicate_input_names_count_once() {
        let out = suggest(
            &c(&["a", "a", "b"]),
            &c(&["a", "a"]),
            PairingOptions::default(),
        );
        assert_eq!(out, vec![m("a"), s("b")]);
    }

    #[test]
    fn empty_inputs_give_nothing() {
        assert!(suggest(&[], &[], PairingOptions::default()).is_empty());
    }

    #[test]
    fn one_empty_side_lists_everything_as_only_in_the_other() {
        let o = PairingOptions::default();
        assert_eq!(suggest(&c(&["b", "a"]), &[], o), vec![s("a"), s("b")]);
        assert_eq!(suggest(&[], &c(&["b", "a"]), o), vec![t("a"), t("b")]);
    }
}
