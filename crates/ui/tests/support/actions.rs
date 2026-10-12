//! Reusable user gestures built from clicks and typing (never from direct state edits).
use super::fixture::Fixture;
use super::screen::{H, dump};
use super::wait::{SHORT, settle, wait_until};
use eframe::egui::{Key, Modifiers};
use egui_kittest::kittest::{NodeT, Queryable};
use mongostudio_ui::app::state::ConnStatus;
use std::time::Duration;

/// Clicks the text box labelled `label`, replaces its content with `text`.
pub fn fill(f: &mut Fixture, label: &str, text: &str) {
    f.harness.get_by_label(label).click();
    settle(&mut f.harness);
    f.harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    settle(&mut f.harness);
    f.harness.get_by_label(label).type_text(text);
    settle(&mut f.harness);
}

/// [`fill`] for the box whose caption is the i18n key `key`.
pub fn fill_key(f: &mut Fixture, key: &str, text: &str) {
    let label = f.labels.t(key);
    fill(f, &label, text);
}

/// The current text of the box labelled `label`.
pub fn text_of(h: &H<'_>, label: &str) -> String {
    h.get_by_label(label).value().unwrap_or_default()
}

pub fn click_key(f: &mut Fixture, key: &str) {
    let label = f.labels.t(key);
    f.click_text(&label);
}

/// Waits (bounded) until `text` is on screen as a label.
#[track_caller]
pub fn wait_shows(f: &mut Fixture, text: &str) {
    let what = format!("{text:?} on screen");
    wait_until(&mut f.harness, &what, SHORT, |h| {
        h.query_all_by_label(text).next().is_some()
    });
}

/// Waits (bounded) until no label equals `text`.
#[track_caller]
pub fn wait_gone(f: &mut Fixture, text: &str) {
    let what = format!("{text:?} to disappear");
    wait_until(&mut f.harness, &what, SHORT, |h| {
        h.query_all_by_label(text).next().is_none()
    });
}

/// Saved connection -> Connect -> the sidebar lists the test database.
#[track_caller]
pub fn connect(f: &mut Fixture) {
    click_key(f, "connections.connect");
    let row = format!("▸ {}", f.db.name);
    wait_shows(f, &row);
    assert_eq!(f.harness.state().state().status, ConnStatus::Connected);
}

/// Expands the test database and opens `collection`; waits for the first document.
#[track_caller]
pub fn open_collection(f: &mut Fixture, collection: &str) {
    let row = format!("▸ {}", f.db.name);
    f.click_text(&row);
    wait_shows(f, collection);
    f.click_text(collection);
    let first = f.labels.t_with("doc.header", &[("n", "1")]);
    wait_shows(f, &first);
}

/// Connect + open `collection`.
#[track_caller]
pub fn browse(f: &mut Fixture, collection: &str) {
    connect(f);
    open_collection(f, collection);
}

/// Right-clicks the row labelled `header` and chooses the entry with i18n key `item_key`.
pub fn choose_from_menu(f: &mut Fixture, header: &str, item_key: &str) {
    f.harness.get_by_label(header).click_secondary();
    settle(&mut f.harness);
    click_key(f, item_key);
}

/// The context-menu entries visible after right-clicking `header` (then closes it with Escape).
pub fn menu_entries(f: &mut Fixture, header: &str, candidates: &[&str]) -> Vec<String> {
    f.harness.get_by_label(header).click_secondary();
    settle(&mut f.harness);
    let shown = candidates
        .iter()
        .filter(|key| {
            let label = f.labels.t(key);
            f.harness.query_all_by_label(&label).next().is_some()
        })
        .map(|key| key.to_string())
        .collect();
    f.harness.key_press(Key::Escape);
    settle(&mut f.harness);
    shown
}

/// True when `first` appears before `second` in on-screen (tree) order.
pub fn appears_before(h: &H<'_>, first: &str, second: &str) -> bool {
    let position = |text: &str| {
        h.root()
            .children_recursive()
            .position(|n| n.accesskit_node().value().as_deref() == Some(text))
    };
    match (position(first), position(second)) {
        (Some(a), Some(b)) => a < b,
        _ => panic!(
            "expected both {first:?} and {second:?} on screen:\n{}",
            dump(h)
        ),
    }
}

/// Bound for a failure that must surface through the runtime's own timeouts.
pub const FAILURE_BOUND: Duration = Duration::from_secs(60);
