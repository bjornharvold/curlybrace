//! Gestures of the Compare workspace, built from clicks only.
use super::actions::{click_key, connect, fill_key, wait_shows};
use super::db::TestDb;
use super::fixture::Fixture;
use super::screen::{H, dump};
use super::wait::{SHORT, settle, wait_until};
use bson::{Document, doc};
use curlybrace_core::compare::protocol::Side;
use curlybrace_core::error::CoreError;
use curlybrace_ui::app::compare::CompareAction;
use curlybrace_ui::app::reducer::Input;
use curlybrace_ui::viewmodel::compare::{EntryId, PairStatus, Pick, SessionPhase, StatusIcon};
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

/// Source and target collections of the test database, added as a pair, then Check, expecting
/// the "Ready" verdict.
#[track_caller]
pub fn pick_pair_and_check(f: &mut Fixture, source: &str, target: &str) {
    pick_collection(f, Side::Source, source);
    pick_collection(f, Side::Target, target);
    add_pair(f);
    check(f);
}

/// Adds the pair the pickers describe to the list.
#[track_caller]
pub fn add_pair(f: &mut Fixture) {
    click_key(f, "compare.pairs.add_pair");
    wait_until(&mut f.harness, "the pair in the list", SHORT, |h| {
        !h.state().state().compare.pairs.entries.is_empty()
    });
}

/// A status as the tables draw it: its icon, then its label.
pub fn status_label(f: &Fixture, icon: StatusIcon, key: &str) -> String {
    format!("{} {}", icon.glyph(), f.labels.t(key))
}

#[track_caller]
pub fn check(f: &mut Fixture) {
    click_key(f, "compare.preflight.check");
    let ok = status_label(f, StatusIcon::Ok, "compare.preflight.entry_ok");
    wait_shows(f, &ok);
}

pub fn set_key(f: &mut Fixture, fields: &str) {
    fill_key(f, "compare.setup.key", fields);
}

/// Clicks Run and waits until the run has produced its outcome (bounded by `limit`).
#[track_caller]
pub fn start_and_wait(f: &mut Fixture, limit: Duration) {
    click_key(f, "compare.preflight.run");
    wait_finished(f, limit);
}

/// The status of the first pair's row in the run overview, if it has run.
pub fn first_status(f: &Fixture) -> Option<PairStatus> {
    let compare = &f.harness.state().state().compare;
    compare.overview.rows.first().map(|row| row.status.clone())
}

#[track_caller]
pub fn wait_finished(f: &mut Fixture, limit: Duration) {
    wait_until(&mut f.harness, "the run to finish", limit, |h| {
        let overview = &h.state().state().compare.overview;
        overview.active.is_none() && !overview.rows.is_empty()
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

/// A second throwaway `cbit_*` database (the target side of the database-level flows), dropped
/// with the guard.
pub struct Scratch(pub TestDb);

impl Scratch {
    pub fn new(f: &Fixture) -> Self {
        Self(TestDb::new(
            f.db.client.clone(),
            f.runtime().handle().clone(),
        ))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        self.0.drop_database();
    }
}

/// Feeds the reducer an action, as a click would, and runs a few frames.
pub fn send(f: &mut Fixture, action: CompareAction) {
    f.harness.state_mut().dispatch(Input::Compare(action));
    settle(&mut f.harness);
}

/// Picks `db` on `side` and waits for the runtime's listing of its collections.
#[track_caller]
pub fn pick_database(f: &mut Fixture, side: Side, db: &str) {
    send(
        f,
        CompareAction::PickSide(side, Pick::Database(db.to_string())),
    );
    let (db, listed) = (db.to_string(), "the collection listing");
    wait_until(&mut f.harness, listed, SHORT, |h| {
        let form = h.state().state().compare.setup.side(side).clone();
        form.database.as_deref() == Some(db.as_str()) && !form.collections.is_empty()
    });
}

/// The list entry whose source collection is `collection`.
pub fn entry_of(f: &Fixture, collection: &str) -> EntryId {
    let compare = &f.harness.state().state().compare;
    let found = compare
        .pairs
        .entries
        .iter()
        .find(|e| e.source.collection == collection);
    found
        .unwrap_or_else(|| panic!("no pair for {collection}"))
        .id
}

/// The overview status of `entry`'s row.
pub fn status_of(f: &Fixture, entry: EntryId) -> PairStatus {
    let overview = &f.harness.state().state().compare.overview;
    let row = overview
        .row_for(entry)
        .unwrap_or_else(|| panic!("no row for entry {entry}"));
    row.status.clone()
}

/// Sends Check and waits until every included entry has a verdict and Run is allowed.
#[track_caller]
pub fn check_all(f: &mut Fixture) {
    send(f, CompareAction::RunCheck);
    wait_until(&mut f.harness, "every check to pass", SHORT, |h| {
        h.state().state().compare.can_start()
    });
}
