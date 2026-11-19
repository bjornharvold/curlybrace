use super::*;

#[test]
fn requests_are_numbered_above_the_base_and_never_reused() {
    let ui = HintListUi::default();
    assert!(ui.wants_list());
    let (ui, first) = ui.requested();
    assert!(is_hint_request(first));
    assert!(first > HINT_REQUEST_BASE, "numbered above the base");
    assert_eq!(ui.list, HintList::Loading { request: first });
    assert!(!ui.wants_list());
    let (_, second) = ui.reset().requested();
    assert!(second > first, "never reused");
    assert!(!is_hint_request(0));
    assert!(!is_hint_request(1_000_000));
}

#[test]
fn only_the_awaited_answer_loads_the_list() {
    let (ui, request) = HintListUi::default().requested();
    let ignored = ui.clone().arrived(request + 1, vec!["x".into()]);
    assert_eq!(ignored, ui);
    let loaded = ui.arrived(request, vec!["a_1".into(), "b_1".into()]);
    assert_eq!(loaded.names(), ["a_1", "b_1"]);
    // An answer after the list is loaded changes nothing.
    assert_eq!(loaded.clone().arrived(request, vec!["z".into()]), loaded);
}

#[test]
fn a_late_failure_of_an_older_request_changes_nothing() {
    let (ui, first) = HintListUi::default().requested();
    let (ui, second) = ui.reset().requested();
    let after = ui.clone().failed(first);
    assert_eq!(after, ui);
    let failed = ui.failed(second);
    assert_eq!(failed.list, HintList::Failed);
    assert!(failed.names().is_empty());
}

#[test]
fn reset_forgets_the_list_but_not_the_counter() {
    let (ui, first) = HintListUi::default().requested();
    let loaded = ui.arrived(first, vec!["a".into()]);
    let reset = loaded.reset();
    assert_eq!(reset.list, HintList::NotAsked);
    let (_, next) = reset.requested();
    assert!(next > first);
}

#[test]
fn retried_only_moves_a_failed_list() {
    let (ui, request) = HintListUi::default().requested();
    assert_eq!(ui.clone().retried(), ui, "loading stays loading");
    let failed = ui.failed(request);
    assert_eq!(failed.retried().list, HintList::NotAsked);
    let loaded = HintListUi::default().from_names(vec!["a".into()]);
    assert_eq!(loaded.clone().retried(), loaded);
    assert_eq!(loaded.names(), ["a"]);
}

#[test]
fn unavailable_fails_without_a_request() {
    let ui = HintListUi::default().unavailable();
    assert_eq!(ui.list, HintList::Failed);
}
