use super::*;

fn sid(n: u128) -> SessionId {
    SessionId::from_u128(n)
}

fn db(session: u128, name: &str) -> NodeRef {
    NodeRef::Database {
        session: sid(session),
        db: name.to_string(),
    }
}

fn coll(session: u128, d: &str, c: &str) -> NodeRef {
    NodeRef::Collection {
        session: sid(session),
        ns: Namespace::new(d, c),
    }
}

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn selection(nodes: &[NodeRef]) -> SidebarSelection {
    nodes
        .iter()
        .cloned()
        .fold(SidebarSelection::default(), SidebarSelection::toggled)
}

/// shop (a, b, c) expanded, crm (x) expanded, then the collapsed `logs`.
fn tree() -> SidebarState {
    SidebarState::default()
        .with_databases(names(&["shop", "crm", "logs"]))
        .toggled("shop")
        .with_collections("shop", names(&["a", "b", "c"]))
        .toggled("crm")
        .with_collections("crm", names(&["x"]))
}

#[test]
fn click_selects_one() {
    let s = selection(&[db(1, "shop"), coll(1, "shop", "a")]).single(coll(1, "shop", "b"));
    assert_eq!(s.nodes(), [coll(1, "shop", "b")]);
    assert!(s.is_selected(&coll(1, "shop", "b")));
    assert!(!s.is_selected(&db(1, "shop")));
}

#[test]
fn cmd_click_toggles() {
    let s = SidebarSelection::default()
        .toggled(coll(1, "shop", "a"))
        .toggled(coll(1, "shop", "c"));
    assert_eq!(s.nodes(), [coll(1, "shop", "a"), coll(1, "shop", "c")]);
    let s = s.toggled(coll(1, "shop", "a"));
    assert_eq!(s.nodes(), [coll(1, "shop", "c")]);
    assert!(s.toggled(coll(1, "shop", "c")).is_empty());
}

#[test]
fn clicking_a_node_of_another_session_starts_a_new_selection() {
    let s = selection(&[coll(1, "shop", "a"), coll(1, "shop", "b")]);
    assert_eq!(
        s.clone().toggled(coll(2, "shop", "a")).nodes(),
        [coll(2, "shop", "a")]
    );
    assert_eq!(s.clone().single(db(2, "crm")).nodes(), [db(2, "crm")]);
    let visible = vec![coll(1, "shop", "a"), coll(2, "shop", "a")];
    let extended = s.extended_to(coll(2, "shop", "a"), &visible);
    assert_eq!(extended.nodes(), [coll(2, "shop", "a")]);
    assert_eq!(extended.session(), Some(sid(2)));
}

#[test]
fn shift_click_selects_range_in_visible_order() {
    let visible = visible_nodes(sid(1), &tree());
    let s = SidebarSelection::default()
        .single(coll(1, "shop", "c"))
        .extended_to(coll(1, "shop", "a"), &visible);
    // Visible order, not click order: a, b, c.
    assert_eq!(
        s.nodes(),
        [
            coll(1, "shop", "a"),
            coll(1, "shop", "b"),
            coll(1, "shop", "c")
        ]
    );
    // The anchor stays: extending again re-computes from `c`.
    let s = s.extended_to(db(1, "crm"), &visible);
    assert_eq!(
        s.nodes(),
        [coll(1, "shop", "c"), db(1, "crm")],
        "range runs from the anchor c to crm"
    );
}

#[test]
fn shift_click_without_an_anchor_selects_the_node_alone() {
    let visible = visible_nodes(sid(1), &tree());
    let s = SidebarSelection::default().extended_to(coll(1, "shop", "b"), &visible);
    assert_eq!(s.nodes(), [coll(1, "shop", "b")]);
}

#[test]
fn visible_nodes_skip_collapsed_and_unloaded_databases() {
    assert_eq!(
        visible_nodes(sid(7), &tree()),
        vec![
            db(7, "shop"),
            coll(7, "shop", "a"),
            coll(7, "shop", "b"),
            coll(7, "shop", "c"),
            db(7, "crm"),
            coll(7, "crm", "x"),
            db(7, "logs"),
        ]
    );
}

#[test]
fn select_applies_the_mode() {
    let visible = visible_nodes(sid(1), &tree());
    let a = coll(1, "shop", "a");
    let c = coll(1, "shop", "c");
    let s = SidebarSelection::default().select(a.clone(), SelectMode::Single, &visible);
    let s = s.select(c.clone(), SelectMode::Toggle, &visible);
    assert_eq!(s.nodes(), [a.clone(), c.clone()]);
    // The toggle moved the anchor to `c`; extending to `a` selects a, b, c.
    let s = s.select(a, SelectMode::Extend, &visible);
    assert_eq!(s.nodes().len(), 3);
}

#[test]
fn without_session_clears_only_that_session() {
    let s = selection(&[db(1, "shop")]);
    assert!(s.clone().without_session(sid(1)).is_empty());
    assert_eq!(s.clone().without_session(sid(2)), s);
}

#[test]
fn a_single_collection_or_several_of_one_database_copy_as_collections() {
    let s = selection(&[coll(1, "shop", "a")]);
    assert_eq!(
        clipboard_from_selection(&s),
        Some(CopyClipboard::Collections {
            session: sid(1),
            db: "shop".into(),
            names: names(&["a"]),
        })
    );
    let s = selection(&[coll(1, "shop", "b"), coll(1, "shop", "a")]);
    assert_eq!(
        clipboard_from_selection(&s),
        Some(CopyClipboard::Collections {
            session: sid(1),
            db: "shop".into(),
            names: names(&["b", "a"]),
        })
    );
}

#[test]
fn a_selected_database_alone_copies_the_database() {
    assert_eq!(
        clipboard_from_selection(&selection(&[db(1, "shop")])),
        Some(CopyClipboard::Database {
            session: sid(1),
            db: "shop".into(),
        })
    );
}

#[test]
fn a_database_plus_its_collections_copies_the_database() {
    let s = selection(&[db(1, "shop"), coll(1, "shop", "a"), coll(1, "shop", "b")]);
    assert_eq!(
        clipboard_from_selection(&s),
        Some(CopyClipboard::Database {
            session: sid(1),
            db: "shop".into(),
        })
    );
}

#[test]
fn mixed_databases_and_collections_copy_only_the_collections_of_one_database() {
    let s = selection(&[db(1, "crm"), coll(1, "shop", "a"), coll(1, "shop", "b")]);
    assert_eq!(
        clipboard_from_selection(&s),
        Some(CopyClipboard::Collections {
            session: sid(1),
            db: "shop".into(),
            names: names(&["a", "b"]),
        })
    );
}

#[test]
fn collections_spanning_two_databases_are_not_copyable() {
    let s = selection(&[coll(1, "shop", "a"), coll(1, "crm", "x")]);
    assert_eq!(clipboard_from_selection(&s), None);
}

#[test]
fn several_databases_and_an_empty_selection_are_not_copyable() {
    assert_eq!(
        clipboard_from_selection(&selection(&[db(1, "shop"), db(1, "crm")])),
        None
    );
    assert_eq!(clipboard_from_selection(&SidebarSelection::default()), None);
}

#[test]
fn server_owned_databases_and_system_collections_are_not_copyable() {
    assert_eq!(
        clipboard_from_selection(&selection(&[db(1, "admin")])),
        None
    );
    assert_eq!(
        clipboard_from_selection(&selection(&[coll(1, "shop", "system.views")])),
        None,
        "nothing left after dropping system collections"
    );
}
