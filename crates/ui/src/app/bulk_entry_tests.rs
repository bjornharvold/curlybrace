//! The entry buttons and the reducer agree: the view enables a button exactly when `Open` would
//! start a flow, and states the reason the reducer refuses for.
use super::bulk_test_support::*;
use crate::viewmodel::BulkEntry;

fn opens(s: AppState) -> bool {
    let (s, _) = act(s, BulkAction::Open(BulkKind::Update));
    s.bulk.is_some()
}

#[test]
fn a_plain_browsing_session_is_available_and_opens() {
    let (s, _) = plain();
    assert_eq!(s.bulk_entry(), BulkEntry::Available);
    assert!(opens(s));
}

#[test]
fn read_only_is_refused_by_both() {
    let (s, _) = browsing(config(false, true));
    assert_eq!(s.bulk_entry(), BulkEntry::ReadOnly);
    assert!(!opens(s));
}

#[test]
fn a_projected_page_is_refused_by_both() {
    let (mut s, _) = plain();
    s.page_query.projection = Some(doc! { "a": 1 });
    assert_eq!(s.bulk_entry(), BulkEntry::Projected);
    assert!(!opens(s));
}

#[test]
fn an_open_flow_or_an_in_flight_write_is_busy_for_both() {
    let (s, _) = plain();
    let (open, _) = act(s, BulkAction::Open(BulkKind::Delete));
    assert_eq!(open.bulk_entry(), BulkEntry::Busy);
    let (again, cmds) = act(open, BulkAction::Open(BulkKind::Update));
    assert!(cmds.is_empty(), "no second flow starts");
    assert_eq!(flow(&again).kind, BulkKind::Delete);

    let (mut s, _) = plain();
    s.pending = Some(Pending {
        ns: ns(),
        editor: None,
    });
    assert_eq!(s.bulk_entry(), BulkEntry::Busy);
    assert!(!opens(s));
}

#[test]
fn a_parked_write_is_busy_for_both() {
    let (s, _) = browsing(config(true, false));
    let (s, _) = update(
        s,
        Input::Editor(crate::viewmodel::EditorAction::Save {
            original_id: None,
            text: "{}".into(),
        }),
    );
    assert!(s.write_warning.is_some());
    assert_eq!(s.bulk_entry(), BulkEntry::Busy);
    assert!(!opens(s));
}
