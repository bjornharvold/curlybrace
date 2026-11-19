//! A write parked for the production warning names its tab, and Copy as code follows the tab in
//! view (#159).
use crate::app::reducer::{Input, update};
use crate::app::state::{AppState, Banner};
use crate::app::tabs::TabId;
use crate::app::tabs::test_support::{act, parked, two_tabs};
use crate::app::write_guard::WriteWarningAction;
use crate::viewmodel::{DocAction, EditorAction, FindAction};
use curlybrace_core::runtime::Command;

#[test]
fn a_confirmed_production_write_whose_tab_closed_sends_nothing() {
    let t = two_tabs();
    let mut s = t.s;
    s.production = true;
    let s = saving_in_production(s, t.second);
    assert!(s.write_warning.is_some(), "the write waits for the warning");
    let s = s.tab_closed(t.second);
    let (s, commands) = update(s, Input::WriteWarning(WriteWarningAction::Confirm));
    assert!(
        commands.is_empty(),
        "nothing is sent for a tab that is gone"
    );
    assert_eq!(s.banner, Some(Banner::WriteNotSentConnectionChanged));
    assert!(s.pending.is_none());
}

#[test]
fn a_confirmed_production_write_whose_tab_is_open_is_sent() {
    let t = two_tabs();
    let mut s = t.s;
    s.production = true;
    let s = saving_in_production(s, t.second);
    let (s, commands) = update(s, Input::WriteWarning(WriteWarningAction::Confirm));
    assert!(matches!(commands.as_slice(), [Command::Insert { .. }]));
    assert_eq!(s.pending.as_ref().and_then(|p| p.tab), Some(t.second));
}

/// An insert saved in `tab` on a production connection: it is parked for the warning.
fn saving_in_production(s: AppState, tab: TabId) -> AppState {
    let (s, _) = act(s, tab, Input::Doc(DocAction::Insert));
    let text = parked(&s, tab)
        .editor
        .as_ref()
        .expect("an editor")
        .text
        .clone();
    let (s, commands) = act(
        s,
        tab,
        Input::Editor(EditorAction::Save {
            original_id: None,
            text,
        }),
    );
    assert!(commands.is_empty(), "parked, not sent");
    s
}

#[test]
fn copy_as_code_closes_when_its_tab_is_not_in_view() {
    let t = two_tabs();
    let (s, _) = update(t.s, Input::Find(FindAction::CopyAsCode));
    assert!(s.code_export.dialog.is_some());
    let (s, _) = update(s, Input::DismissBanner);
    assert!(s.code_export.dialog.is_some(), "still in view");
    let s = s.tab_current(t.second);
    let (s, _) = update(s, Input::DismissBanner);
    assert!(
        s.code_export.dialog.is_none(),
        "its tab is not in view any more"
    );
}
