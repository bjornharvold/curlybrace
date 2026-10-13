//! Gestures of the Compare workspace, built from clicks only.
use super::actions::{click_key, connect, fill_key, wait_shows};
use super::fixture::Fixture;
use super::screen::{H, dump};
use super::wait::{SHORT, settle, wait_until};
use bson::{Document, doc};
use curlybrace_core::compare::protocol::Side;
use curlybrace_core::error::CoreError;
use curlybrace_ui::viewmodel::compare::{RunView, SessionPhase};
use eframe::egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use std::time::Duration;

/// Bound for scanning the large collections of the cancel flow to the end.
pub const LONG: Duration = Duration::from_secs(180);

/// Connects the saved connection, switches to Compare and opens the session on it (both sides).
#[track_caller]
pub fn open_compare(f: &mut Fixture) {
    connect(f);
    click_key(f, "workspace.compare");
    click_key(f, "compare.session.open");
    wait_until(&mut f.harness, "the compare session", SHORT, |h| {
        matches!(
            h.state().state().compare.session,
            Some(SessionPhase::Ready(_))
        )
    });
}

/// Opens the combo box named by `caption_key` on `side` (source comes first on screen) and
/// chooses the entry `entry` below it.
#[track_caller]
fn choose(f: &mut Fixture, side: Side, caption_key: &str, entry: &str) {
    let caption = f.labels.t(caption_key);
    let nth = usize::from(side == Side::Target);
    let combo = f
        .harness
        .get_all_by_role_and_label(Role::ComboBox, &caption)
        .nth(nth)
        .unwrap_or_else(|| panic!("no {caption:?} box for {side:?}:\n{}", dump(&f.harness)));
    let bottom = combo.rect().bottom();
    combo.click();
    settle(&mut f.harness);
    let option = f
        .harness
        .query_all_by_label(entry)
        .find(|n| n.accesskit_node().role() != Role::ComboBox && n.rect().top() >= bottom)
        .unwrap_or_else(|| panic!("no entry {entry:?}:\n{}", dump(&f.harness)));
    option.click();
    settle(&mut f.harness);
}

/// Picks `collection` of the test database on `side`.
#[track_caller]
pub fn pick_collection(f: &mut Fixture, side: Side, collection: &str) {
    let db = f.db.name.clone();
    choose(f, side, "compare.setup.database", &db);
    let wanted = collection.to_string();
    let side_idx = side;
    wait_until(&mut f.harness, "the collection list", SHORT, |h| {
        let form = h.state().state().compare.setup.side(side_idx).clone();
        form.collections.iter().any(|(name, _)| *name == wanted)
    });
    choose(f, side, "compare.setup.collection", collection);
}

/// Source and target collections of the test database, then Check, expecting the "ready" line.
#[track_caller]
pub fn pick_pair_and_check(f: &mut Fixture, source: &str, target: &str) {
    pick_collection(f, Side::Source, source);
    pick_collection(f, Side::Target, target);
    check(f);
}

#[track_caller]
pub fn check(f: &mut Fixture) {
    click_key(f, "compare.preflight.check");
    let ok = f.labels.t("compare.preflight.ok");
    wait_shows(f, &ok);
}

pub fn set_key(f: &mut Fixture, fields: &str) {
    fill_key(f, "compare.setup.key", fields);
}

/// Clicks Start and waits until the run has produced its outcome (bounded by `limit`).
#[track_caller]
pub fn start_and_wait(f: &mut Fixture, limit: Duration) {
    click_key(f, "compare.run.start");
    wait_finished(f, limit);
}

#[track_caller]
pub fn wait_finished(f: &mut Fixture, limit: Duration) {
    wait_until(&mut f.harness, "the run to finish", limit, |h| {
        matches!(
            h.state().state().compare.run_view(),
            RunView::Finished { .. }
        )
    });
}

/// The localized text of an error as the run panel shows it.
pub fn error_text(f: &Fixture, error: &CoreError) -> String {
    f.labels.i18n().error(error)
}

/// `count` documents `{_id: i, pad}` with a payload of about `pad` bytes (for the large flows).
pub fn padded(range: std::ops::Range<i32>, pad: usize) -> Vec<Document> {
    let filler = "x".repeat(pad);
    range
        .map(|i| doc! {"_id": i, "n": i, "pad": filler.as_str()})
        .collect()
}

pub fn is_on_screen(h: &H<'_>, text: &str) -> bool {
    h.query_all_by_label(text).next().is_some()
}

/// Waits (bounded) until the text of the i18n key `key` is on screen.
#[track_caller]
pub fn wait_shows_key(f: &mut Fixture, key: &str) {
    let text = f.labels.t(key);
    wait_shows(f, &text);
}
