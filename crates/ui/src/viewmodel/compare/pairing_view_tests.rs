use super::*;
use crate::viewmodel::compare::pairs::PairList;
use crate::viewmodel::compare::setup::SideForm;
use curlybrace_core::compare::protocol::Side;
use curlybrace_core::store::CollectionKind;

type Listing = Vec<(String, CollectionKind)>;

fn coll(names: &[&str]) -> Listing {
    names
        .iter()
        .map(|n| (n.to_string(), CollectionKind::Collection))
        .collect()
}

fn setup(source: Listing, target: Listing) -> SetupForm {
    let side = |db: &str, entries| {
        SideForm::default()
            .with_database(db)
            .with_collections(db, entries)
    };
    SetupForm::default()
        .with_side(Side::Source, side("a", source))
        .with_side(Side::Target, side("b", target))
}

fn matched(name: &str, paired: bool) -> PairingRow {
    PairingRow::Matched {
        name: name.to_string(),
        paired,
    }
}

fn lonely(side: Side, name: &str) -> PairingRow {
    PairingRow::NoCounterpart {
        side,
        name: name.to_string(),
    }
}

fn paired_users() -> PairList {
    PairList::default().added(Namespace::new("a", "users"), Namespace::new("b", "users"))
}

#[test]
fn rows_list_matched_then_no_counterpart_in_core_order() {
    let form = setup(
        coll(&["users", "orders", "zeta"]),
        coll(&["users", "orders", "alpha"]),
    );
    let rows = DatabasePairing::default().rows(&form, &PairList::default());
    assert_eq!(
        rows,
        vec![
            matched("orders", false),
            matched("users", false),
            lonely(Side::Source, "zeta"),
            lonely(Side::Target, "alpha"),
        ]
    );
}

#[test]
fn matched_row_is_marked_paired_once_added() {
    let form = setup(coll(&["users", "orders"]), coll(&["users", "orders"]));
    let rows = DatabasePairing::default().rows(&form, &paired_users());
    assert_eq!(rows, vec![matched("orders", false), matched("users", true)]);
}

#[test]
fn include_system_toggles_system_collections() {
    let form = setup(
        coll(&["users", "system.profile"]),
        coll(&["users", "system.profile"]),
    );
    let list = PairList::default();
    let hidden = DatabasePairing::default().rows(&form, &list);
    assert_eq!(hidden, vec![matched("users", false)]);
    let shown = DatabasePairing {
        include_system: true,
    }
    .rows(&form, &list);
    assert_eq!(shown.len(), 2);
}

#[test]
fn matched_to_add_skips_already_paired_names() {
    let form = setup(
        coll(&["users", "orders", "solo"]),
        coll(&["users", "orders"]),
    );
    let adds = DatabasePairing::default().matched_to_add(&form, &paired_users());
    assert_eq!(
        adds,
        vec![(Namespace::new("a", "orders"), Namespace::new("b", "orders"))]
    );
}

#[test]
fn views_are_not_offered() {
    let mut source = coll(&["users"]);
    source.push(("v".to_string(), CollectionKind::View));
    let form = setup(source, coll(&["users", "v"]));
    let rows = DatabasePairing::default().rows(&form, &PairList::default());
    assert_eq!(rows, vec![matched("users", false)]);
}

#[test]
fn no_database_picked_gives_no_rows() {
    let pairing = DatabasePairing::default();
    let list = PairList::default();
    assert!(pairing.rows(&SetupForm::default(), &list).is_empty());
    let one_side = SetupForm::default().with_side(
        Side::Source,
        SideForm::default()
            .with_database("a")
            .with_collections("a", coll(&["users"])),
    );
    assert!(pairing.rows(&one_side, &list).is_empty());
    assert!(pairing.matched_to_add(&one_side, &list).is_empty());
}
