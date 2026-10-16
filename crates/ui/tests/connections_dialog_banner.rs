//! The edit dialog repeats the app banner only when it is new since the dialog opened: a banner
//! that was already showing (a past connect failure, a persistence notice) is about something
//! else and stays in its normal place.
mod conn_harness;

use conn_harness::Harness;
use curlybrace_core::error::CoreError;
use curlybrace_ui::app::persistence::PersistGuard;
use curlybrace_ui::app::state::Banner;
use curlybrace_ui::viewmodel::ConnectionForm;
use curlybrace_ui::views::connections::ConnectionAction;

fn connect_failure() -> Banner {
    Banner::Failure {
        context: "connect",
        error: CoreError::AuthFailed,
    }
}

fn keychain_failure() -> Banner {
    Banner::Failure {
        context: "save_connection",
        error: CoreError::Secret("keychain said no".into()),
    }
}

fn open_dialog(h: &mut Harness) {
    h.app.state_mut().conn.editing = true;
    h.frame();
}

fn shown(h: &Harness) -> bool {
    !h.app.state().conn.dialog_banner.is_empty()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_banner_that_was_showing_before_the_dialog_opened_is_not_repeated() {
    let mut h = Harness::new(Vec::new());
    h.app.state_mut().banner = Some(connect_failure());
    h.frame();
    open_dialog(&mut h);
    h.frame();
    assert!(!shown(&h), "stale banner repeated in the dialog");
    assert!(h.app.state().banner.is_some(), "it stays in its place");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_banner_raised_while_the_dialog_is_open_is_repeated() {
    let mut h = Harness::new(Vec::new());
    open_dialog(&mut h);
    h.app.state_mut().banner = Some(connect_failure());
    h.frame();
    assert!(shown(&h));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_banner_replaced_by_a_different_one_while_open_is_shown() {
    let mut h = Harness::new(Vec::new());
    h.app.state_mut().banner = Some(connect_failure());
    open_dialog(&mut h);
    assert!(!shown(&h));
    h.app.state_mut().banner = Some(keychain_failure());
    h.frame();
    assert!(shown(&h));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn closing_and_reopening_with_a_banner_present_does_not_show_it_again() {
    let mut h = Harness::new(Vec::new());
    open_dialog(&mut h);
    h.app.state_mut().banner = Some(keychain_failure());
    h.frame();
    assert!(shown(&h));
    h.app.state_mut().conn.close_edit();
    h.frame();
    assert!(!shown(&h), "a closed dialog shows nothing");
    open_dialog(&mut h);
    assert!(!shown(&h), "the banner predates this opening");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_save_failure_raised_from_inside_the_dialog_still_shows() {
    let mut h = Harness::new(Vec::new());
    // A stale unrelated banner first, then the dialog, then a real failing save.
    h.app.state_mut().banner = Some(connect_failure());
    open_dialog(&mut h);
    // The real refusal path: a save while the file may not be overwritten raises a persistence
    // banner and leaves the dialog open (a failed file write closes it instead, so the banner
    // then shows in its normal place).
    h.app.state_mut().persist = PersistGuard::blocked();
    let form = ConnectionForm {
        name: "Doomed".into(),
        uri: "mongodb://localhost:27017".into(),
        ..ConnectionForm::default()
    };
    h.app
        .dispatch_connection(ConnectionAction::Save(Box::new(form)));
    h.frame();
    assert!(
        matches!(h.app.state().banner, Some(Banner::Persist(_))),
        "the save was refused with a persistence banner: {:?}",
        h.app.state().banner
    );
    assert!(h.app.state().conn.editing, "a failed save keeps the dialog");
    assert!(shown(&h), "the save failure belongs to the dialog");
}
