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
    // The database lists of both sides arrive after Ready; a picker opened earlier is empty.
    wait_until(&mut f.harness, "both database lists", SHORT, |h| {
        let setup = &h.state().state().compare.setup;
        !setup.side(Side::Source).databases.is_empty()
            && !setup.side(Side::Target).databases.is_empty()
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

// ---- Apply gestures: clicks and typing only ----

/// Clicks the results tab `category_key` (for example `compare.category.only_in_source`) whose
/// title shows `count`, and waits until that tab's rows are loaded.
#[track_caller]
pub fn open_tab(f: &mut Fixture, category_key: &str, count: usize) {
    let name = f.labels.t(category_key);
    let title = f.labels.t_with(
        "compare.results.tab",
        &[("name", &name), ("count", &count.to_string())],
    );
    f.click_text(&title);
    wait_until(&mut f.harness, "the tab's rows", SHORT, |h| {
        let results = h.state().state().compare.open_results().cloned();
        results.is_some_and(|r| r.total() == count && !r.is_loading() && !r.rows.is_empty())
    });
}

/// Ticks the checkbox of each row, named by its key text.
#[track_caller]
pub fn tick_rows(f: &mut Fixture, keys: &[&str]) {
    for key in keys {
        let checkbox = f
            .harness
            .get_all_by_role_and_label(Role::CheckBox, key)
            .next()
            .unwrap_or_else(|| panic!("no checkbox for row {key:?}:\n{}", dump(&f.harness)));
        checkbox.click();
        settle(&mut f.harness);
    }
}

/// Clicks "Select all N in this tab" (every row of the tab, loaded or not).
#[track_caller]
pub fn select_all_in_tab(f: &mut Fixture, count: usize) {
    let label = f.labels.t_with(
        "compare.results.select_all",
        &[("count", &count.to_string())],
    );
    f.click_text(&label);
}

/// Clicks the action button (`compare.apply.action.copy_to_target` and the like) of the bar.
pub fn click_action(f: &mut Fixture, action_key: &str) {
    click_key(f, action_key);
}

/// Clicks "Review changes..." and waits for the dialog.
#[track_caller]
pub fn open_review(f: &mut Fixture) {
    click_key(f, "compare.apply.pending.review");
    wait_until(&mut f.harness, "the review dialog", SHORT, |h| {
        h.state().state().compare.apply.review().is_some()
    });
}

/// Types `text` into the confirmation box of the review dialog.
pub fn type_confirmation(f: &mut Fixture, text: &str) {
    fill_key(f, "compare.apply.review.confirm_prompt", text);
}

/// Whether the dialog's Apply button is enabled right now.
#[track_caller]
pub fn apply_enabled(f: &Fixture) -> bool {
    let label = f.labels.t("compare.apply.review.apply");
    let button = f.harness.get_by_label(&label);
    !button.accesskit_node().is_disabled()
}

/// Clicks Apply and waits until the Finished panel is up (bounded by `limit`).
#[track_caller]
pub fn confirm_and_wait(f: &mut Fixture, limit: Duration) {
    click_key(f, "compare.apply.review.apply");
    wait_until(&mut f.harness, "the apply to finish", limit, |h| {
        h.state().state().compare.apply.is_finished()
    });
    wait_shows_key(f, "compare.apply.finished.title");
}

/// Dismisses the Finished panel.
pub fn dismiss_result(f: &mut Fixture) {
    click_key(f, "compare.apply.finished.dismiss");
}

/// Re-runs the pair of `collection` (Re-run, Check, Run) and returns its new summary.
#[track_caller]
pub fn rerun(f: &mut Fixture, collection: &str) -> curlybrace_core::compare::engine::Summary {
    let entry = entry_of(f, collection);
    send(f, CompareAction::Rerun(entry));
    check_all(f);
    send(f, CompareAction::Start);
    wait_finished(f, SHORT);
    match status_of(f, entry) {
        PairStatus::Completed(summary) => summary,
        other => panic!("{collection} should have completed, found {other:?}"),
    }
}

/// Clicks the key line of a results row (not its checkbox, which carries the same name) to
/// open the row's detail.
#[track_caller]
pub fn open_row(f: &mut Fixture, key: &str) {
    let line = f
        .harness
        .query_all_by_label(key)
        .find(|n| n.accesskit_node().role() != Role::CheckBox)
        .unwrap_or_else(|| panic!("no row {key:?}:\n{}", dump(&f.harness)));
    line.click();
    settle(&mut f.harness);
}

// ---- Field-level gestures ----

/// Opens the detail of the row `key` and waits until the runtime has answered with its changes.
#[track_caller]
pub fn open_detail(f: &mut Fixture, key: &str) {
    open_row(f, key);
    wait_until(&mut f.harness, "the document's detail", SHORT, |h| {
        let detail = &h.state().state().compare.detail;
        detail
            .as_ref()
            .is_some_and(|d| !d.loading && !d.changes.is_empty())
    });
}

/// The field checkbox named by the field path text (for example `address.city`, `tags[1]`).
#[track_caller]
fn field_checkbox<'a>(f: &'a Fixture, path: &'a str) -> egui_kittest::Node<'a> {
    f.harness
        .get_all_by_role_and_label(Role::CheckBox, path)
        .next()
        .unwrap_or_else(|| panic!("no field checkbox {path:?}:\n{}", dump(&f.harness)))
}

/// Ticks the checkbox of each field of the open detail, named by its path label.
#[track_caller]
pub fn tick_fields(f: &mut Fixture, paths: &[&str]) {
    for path in paths {
        field_checkbox(f, path).click();
        settle(&mut f.harness);
    }
}

/// Whether the field checkbox `path` is disabled right now.
#[track_caller]
pub fn field_disabled(f: &Fixture, path: &str) -> bool {
    field_checkbox(f, path).accesskit_node().is_disabled()
}

/// Whether the field checkbox `path` is ticked right now.
#[track_caller]
pub fn field_ticked(f: &Fixture, path: &str) -> bool {
    use eframe::egui::accesskit::Toggled;
    field_checkbox(f, path).accesskit_node().toggled() == Some(Toggled::True)
}

/// Clicks "Copy fields to target" (or "... to source") of the detail pane.
#[track_caller]
pub fn copy_fields(f: &mut Fixture, to_target: bool) {
    let key = if to_target {
        "compare.detail.copy_fields_to_target"
    } else {
        "compare.detail.copy_fields_to_source"
    };
    click_key(f, key);
}

/// Whether the detail pane's queue button for the direction is disabled right now.
#[track_caller]
pub fn copy_fields_disabled(f: &Fixture, to_target: bool) -> bool {
    let key = if to_target {
        "compare.detail.copy_fields_to_target"
    } else {
        "compare.detail.copy_fields_to_source"
    };
    let label = f.labels.t(key);
    let button = f.harness.get_by_label(&label);
    button.accesskit_node().is_disabled()
}

/// Queues the ticked fields of the open detail towards the target, reviews and applies, and
/// waits for the Finished panel.
#[track_caller]
pub fn apply_ticked_fields(f: &mut Fixture, to_target: bool) {
    copy_fields(f, to_target);
    open_review(f);
    confirm_and_wait(f, SHORT);
}

/// Whether the open detail lists a field checkbox for `path`.
pub fn has_field(f: &Fixture, path: &str) -> bool {
    let mut found = f.harness.query_all_by_role_and_label(Role::CheckBox, path);
    found.next().is_some()
}
