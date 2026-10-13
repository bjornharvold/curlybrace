use super::*;
use crate::viewmodel::compare::setup::{ScopeKind, SideForm};
use curlybrace_core::compare::protocol::Side;
use curlybrace_core::store::CollectionKind;

fn ns(db: &str, coll: &str) -> Namespace {
    Namespace::new(db, coll)
}

fn defaults() -> SetupForm {
    let side = |db: &str| {
        SideForm::default()
            .with_database(db)
            .with_collections(db, vec![("users".to_string(), CollectionKind::Collection)])
    };
    SetupForm::default()
        .with_side(Side::Source, side("a"))
        .with_side(Side::Target, side("b"))
}

fn list_of_two() -> PairList {
    PairList::default()
        .added(ns("a", "users"), ns("b", "users"))
        .added(ns("a", "orders"), ns("b", "orders"))
}

fn rules_only() -> RulesForm {
    RulesForm {
        scope: ScopeKind::Only,
        paths_text: "name".to_string(),
        ..RulesForm::default()
    }
}

#[test]
fn added_assigns_stable_increasing_ids() {
    let list = list_of_two();
    let ids: Vec<EntryId> = list.entries.iter().map(|e| e.id).collect();
    assert_eq!(ids.len(), 2);
    assert!(ids[0] < ids[1]);
    let list = list.removed(ids[0]).added(ns("a", "c"), ns("b", "c"));
    let third = list.entries.last().unwrap().id;
    assert!(third > ids[1], "ids are never reused");
    assert!(list.entries.iter().all(|e| e.included));
}

#[test]
fn added_ignores_an_exact_duplicate_pair() {
    let before = list_of_two();
    let next = before.next_entry;
    let list = before.added(ns("a", "users"), ns("b", "users"));
    assert_eq!(list.entries.len(), 2);
    assert_eq!(list.next_entry, next);
    let crossed = list.added(ns("a", "users"), ns("b", "orders"));
    assert_eq!(crossed.entries.len(), 3);
}

#[test]
fn removed_clears_editing_when_it_was_the_edited_entry() {
    let list = list_of_two();
    let (first, second) = (list.entries[0].id, list.entries[1].id);
    let editing = list.editing(Some(first));
    assert_eq!(editing.clone().removed(second).editing, Some(first));
    let gone = editing.removed(first);
    assert_eq!(gone.editing, None);
    assert!(gone.get(first).is_none());
}

#[test]
fn defaults_apply_to_entries_without_overrides() {
    let list = list_of_two();
    let entry = &list.entries[0];
    let changed = defaults().with_key_text("code");
    let spec = entry.spec(7, &changed, false).unwrap();
    assert_eq!(spec.key.fields, vec!["code".to_string()]);
}

#[test]
fn changing_a_default_never_overwrites_an_override() {
    let overrides = [
        PairOverrides {
            key_text: Some("sku".to_string()),
            ..PairOverrides::default()
        },
        PairOverrides {
            source_filter: Some(r#"{"s": 1}"#.to_string()),
            ..PairOverrides::default()
        },
        PairOverrides {
            target_filter: Some(r#"{"t": 1}"#.to_string()),
            ..PairOverrides::default()
        },
        PairOverrides {
            rules: Some(rules_only()),
            ..PairOverrides::default()
        },
    ];
    for o in overrides {
        let list = list_of_two();
        let first = list.entries[0].id;
        let list = list.with_overrides(first, o.clone());
        let base = defaults();
        let changed = SetupForm {
            source: base.source.with_filter(r#"{"d": 1}"#),
            target: base.target.with_filter(r#"{"d": 2}"#),
            ..base
        }
        .with_key_text("code")
        .with_rules(RulesForm {
            ignore_array_order: true,
            ..RulesForm::default()
        });
        let own = list.entries[0].spec(1, &changed, false).unwrap();
        let sibling = list.entries[1].spec(2, &changed, false).unwrap();
        assert_eq!(sibling.key.fields, vec!["code".to_string()]);
        assert_eq!(sibling.source_filter, bson::doc! {"d": 1});
        assert_eq!(sibling.target_filter, bson::doc! {"d": 2});
        assert!(sibling.rules.ignore_array_order);
        if o.key_text.is_some() {
            assert_eq!(own.key.fields, vec!["sku".to_string()]);
        }
        if o.source_filter.is_some() {
            assert_eq!(own.source_filter, bson::doc! {"s": 1});
        }
        if o.target_filter.is_some() {
            assert_eq!(own.target_filter, bson::doc! {"t": 1});
        }
        if o.rules.is_some() {
            assert!(!own.rules.ignore_array_order);
            assert_eq!(
                own.rules.field_scope,
                curlybrace_core::compare::rules::FieldScope::Only(vec!["name".to_string()])
            );
        }
    }
}

#[test]
fn is_customised_reports_any_override_and_none_for_default_entries() {
    assert!(!PairOverrides::default().is_customised());
    let each = [
        PairOverrides {
            key_text: Some(String::new()),
            ..PairOverrides::default()
        },
        PairOverrides {
            source_filter: Some(String::new()),
            ..PairOverrides::default()
        },
        PairOverrides {
            target_filter: Some(String::new()),
            ..PairOverrides::default()
        },
        PairOverrides {
            rules: Some(RulesForm::default()),
            ..PairOverrides::default()
        },
    ];
    assert!(each.iter().all(PairOverrides::is_customised));
}

#[test]
fn spec_uses_the_given_pair_id_and_each_entrys_namespaces() {
    let list = list_of_two();
    let spec = list.entries[1].spec(42, &defaults(), false).unwrap();
    assert_eq!(spec.id, 42);
    assert_eq!(spec.source, ns("a", "orders"));
    assert_eq!(spec.target, ns("b", "orders"));
}

#[test]
fn same_collection_only_raised_on_one_connection() {
    let list = PairList::default().added(ns("a", "users"), ns("a", "users"));
    let entry = &list.entries[0];
    assert_eq!(
        entry.spec(1, &defaults(), true).unwrap_err(),
        SetupError::SameCollection
    );
    assert_eq!(
        entry.issues(&defaults(), true),
        vec![SetupError::SameCollection]
    );
    assert!(entry.spec(1, &defaults(), false).is_ok());
    // Different filters make it a real comparison even on one connection.
    let filtered = list.clone().with_overrides(
        entry.id,
        PairOverrides {
            target_filter: Some(r#"{"x": 1}"#.to_string()),
            ..PairOverrides::default()
        },
    );
    assert!(filtered.entries[0].spec(1, &defaults(), true).is_ok());
}

#[test]
fn invalid_override_key_reports_key_error_for_that_entry_only() {
    let list = list_of_two();
    let first = list.entries[0].id;
    let list = list.with_overrides(
        first,
        PairOverrides {
            key_text: Some("a,,b".to_string()),
            ..PairOverrides::default()
        },
    );
    assert_eq!(
        list.entries[0].issues(&defaults(), false),
        vec![SetupError::Key]
    );
    assert_eq!(
        list.entries[0].spec(1, &defaults(), false).unwrap_err(),
        SetupError::Key
    );
    assert!(list.entries[1].issues(&defaults(), false).is_empty());
}

#[test]
fn an_entry_in_another_database_drops_the_listing_and_still_resolves() {
    let list = PairList::default().added(ns("other", "users"), ns("b", "users"));
    let defaults = defaults();
    let effective = list.entries[0].effective(&defaults);
    assert_eq!(effective.source.database.as_deref(), Some("other"));
    assert!(effective.source.collections.is_empty());
    assert_eq!(effective.target.collections, defaults.target.collections);
    let spec = list.entries[0].spec(3, &defaults, false).unwrap();
    assert_eq!(spec.source, ns("other", "users"));
}

#[test]
fn only_included_unticks_the_others() {
    let list = list_of_two().added(ns("a", "c"), ns("b", "c"));
    let keep = list.entries[1].id;
    let list = list.only_included(keep);
    let flags: Vec<bool> = list.entries.iter().map(|e| e.included).collect();
    assert_eq!(flags, vec![false, true, false]);
    let first = list.entries[0].id;
    let list = list.included(first, true);
    assert!(list.entries[0].included);
}

#[test]
fn has_pair_knows_the_exact_namespaces() {
    let list = list_of_two();
    assert!(list.has_pair(&ns("a", "users"), &ns("b", "users")));
    assert!(!list.has_pair(&ns("a", "users"), &ns("b", "orders")));
}

#[test]
fn overrides_are_set_and_cleared_one_field_at_a_time() {
    let o = PairOverrides::default()
        .with_key_text(Some("sku".into()))
        .with_filter(Side::Target, Some("{}".into()))
        .with_rules(Some(rules_only()));
    assert_eq!(o.key_text.as_deref(), Some("sku"));
    assert_eq!(o.target_filter.as_deref(), Some("{}"));
    assert_eq!(o.source_filter, None);
    assert!(o.is_customised());
    let cleared = o
        .with_key_text(None)
        .with_filter(Side::Target, None)
        .with_rules(None);
    assert!(!cleared.is_customised());
}

#[test]
fn issues_name_a_view_on_either_side_without_cloning_the_listing() {
    let side = |db: &str| {
        SideForm::default().with_database(db).with_collections(
            db,
            vec![
                ("users".to_string(), CollectionKind::Collection),
                ("v".to_string(), CollectionKind::View),
            ],
        )
    };
    let d = SetupForm::default()
        .with_side(Side::Source, side("a"))
        .with_side(Side::Target, side("b"));
    let list = PairList::default().added(ns("a", "v"), ns("b", "v"));
    let issues = list.entries[0].issues(&d, false);
    assert_eq!(
        issues,
        vec![
            SetupError::ViewSelected(Side::Source),
            SetupError::ViewSelected(Side::Target)
        ]
    );
    // A view of another database than the picked one is not known to be a view.
    let other = PairList::default().added(ns("z", "v"), ns("b", "users"));
    assert_eq!(other.entries[0].issues(&d, false), vec![]);
}
