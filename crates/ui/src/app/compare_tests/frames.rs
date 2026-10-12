//! Headless frames of the Compare workspace in every phase: nothing panics, an idle frame asks
//! for nothing, and every label is a translated key. Rendering uses a pseudo-locale whose every
//! string is wrapped in guillemets, so a literal English string shows up as unwrapped text.
use super::*;
use crate::I18n;
use crate::views::compare::show_compare;
use bson::doc;
use curlybrace_core::compare::diff::{ChangeKind, FieldChange};
use eframe::egui;

const OPEN: char = '«';
const MISSING: char = '⟦';

/// Wraps every string of `en.json` in guillemets; `{param}` placeholders are kept.
fn pseudo_locale() -> I18n {
    fn wrap(value: serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::String(s) => serde_json::Value::String(format!("«{s}»")),
            serde_json::Value::Object(map) => {
                serde_json::Value::Object(map.into_iter().map(|(k, v)| (k, wrap(v))).collect())
            }
            other => other,
        }
    }
    let en: serde_json::Value = serde_json::from_str(include_str!("../../../locales/en.json"))
        .expect("en.json is valid JSON");
    I18n::from_json(&wrap(en).to_string()).expect("pseudo locale parses")
}

fn collect(shape: &egui::Shape, out: &mut Vec<String>) {
    match shape {
        egui::Shape::Text(text) => out.push(text.galley.text().to_string()),
        egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
        _ => {}
    }
}

/// Everything drawn for `state` after the layout settled, and the action of the last frame.
fn render(state: &AppState) -> (Vec<String>, Option<CompareAction>) {
    render_on(&egui::Context::default(), state)
}

fn frame_texts(ctx: &egui::Context, state: &AppState) -> Vec<String> {
    render_on(ctx, state).0
}

fn render_on(ctx: &egui::Context, state: &AppState) -> (Vec<String>, Option<CompareAction>) {
    let i18n = pseudo_locale();
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1100.0, 900.0),
        )),
        ..Default::default()
    };
    let mut texts = Vec::new();
    let mut action = None;
    for _ in 0..3 {
        texts.clear();
        let output = ctx.run(input.clone(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                action = show_compare(ui, &state.compare, &state.conn, &i18n);
            });
        });
        output
            .shapes
            .iter()
            .for_each(|s| collect(&s.shape, &mut texts));
    }
    (texts, action)
}

/// Texts that are data (names, keys, JSON, numbers), not UI strings.
fn is_data(text: &str) -> bool {
    let data = [
        "a", "b", "users", "_id", "0", "1", "2", "3", "x", "name", "extra", "sku",
    ];
    data.contains(&text) || text.starts_with(['{', '"']) || text.chars().all(|c| !c.is_alphabetic())
}

fn assert_translated(state: &AppState, label: &str) -> Vec<String> {
    let (texts, action) = render(state);
    assert!(!texts.is_empty(), "{label}: nothing was drawn");
    assert_eq!(action, None, "{label}: an idle frame must ask for nothing");
    for text in &texts {
        assert!(!text.contains(MISSING), "{label}: missing key in {text:?}");
        assert!(
            text.contains(OPEN) || is_data(text),
            "{label}: untranslated text {text:?}"
        );
    }
    texts
}

fn has(texts: &[String], needle: &str) -> bool {
    texts.iter().any(|t| t.contains(needle))
}

fn workspace(state: AppState) -> AppState {
    AppState {
        workspace: Workspace::Compare,
        ..state
    }
}

#[test]
fn no_session_offers_the_connections_and_nothing_else() {
    let (state, _, _) = two_connections();
    let texts = assert_translated(&workspace(state), "no session");
    assert!(has(&texts, "Connect"));
    assert!(!has(&texts, "Start"));
}

#[test]
fn no_saved_connections_say_what_to_do_first() {
    let texts = assert_translated(&workspace(AppState::default()), "no connections");
    assert!(has(&texts, "Save a connection first"));
}

#[test]
fn opening_shows_progress_text() {
    let (state, a, b) = two_connections();
    let (state, _) = act(state, open_action(a, b));
    let texts = assert_translated(&workspace(state), "opening");
    assert!(has(&texts, "Connecting to both servers"));
}

#[test]
fn ready_with_an_empty_form_lists_the_missing_picks() {
    let (state, _, _) = opened();
    let texts = assert_translated(&workspace(state), "empty form");
    assert!(has(
        &texts,
        "Pick a database and a collection on both sides"
    ));
    assert!(has(&texts, "Rules"));
}

#[test]
fn a_database_with_no_listed_collections_offers_reload() {
    let (state, _, _) = opened();
    let (state, _) = act(
        state,
        CompareAction::PickSide(Side::Source, Pick::Database("a".into())),
    );
    let texts = assert_translated(&workspace(state), "reload");
    assert!(has(&texts, "Reload"));
}

#[test]
fn a_ready_form_with_a_fresh_check_renders() {
    let texts = assert_translated(&workspace(checked_state()), "checked");
    assert!(has(&texts, "Ready to compare"));
    assert!(has(&texts, "Source»: 10 documents, «key is indexed"));
    assert!(has(&texts, "Target»: 10 documents, «key is indexed"));
}

#[test]
fn counts_disappear_when_the_form_is_edited() {
    let (state, _) = act(checked_state(), CompareAction::SetKey("sku".into()));
    let texts = assert_translated(&workspace(state), "edited after check");
    assert!(!has(&texts, "documents,"));
}

#[test]
fn the_blocked_side_shows_its_missing_index() {
    let blocked = Preflight {
        unindexed_too_large: true,
        source_index: IndexStatus::NotIndexed,
        ..indexed(60_000)
    };
    let (state, _) = act(picked_state(), CompareAction::RunCheck);
    let (state, _) = update(state, checked_event(blocked));
    let texts = assert_translated(&workspace(state), "blocked sizes");
    assert!(has(
        &texts,
        "Source»: 60000 documents, «no index on the key fields"
    ));
    assert!(has(&texts, "Target»: 60000 documents, «key is indexed"));
}

#[test]
fn the_pickers_follow_the_open_pair_when_it_changes_behind_their_back() {
    let (state, a, b) = two_connections();
    let (opened_ab, _) = act(state, open_action(a, b));
    let (reopened, _) = act(
        AppState::new(opened_ab.conn.connections.clone()),
        open_action(b, a),
    );
    let ctx = egui::Context::default();
    let names = |texts: &[String]| -> Vec<String> {
        texts
            .iter()
            .filter(|t| *t == "a" || *t == "b")
            .cloned()
            .collect()
    };
    let first = frame_texts(&ctx, &workspace(opened_ab));
    assert_eq!(names(&first), vec!["a", "b"], "{first:?}");
    // Same egui context, so the first frame's picks are still in memory.
    let second = frame_texts(&ctx, &workspace(reopened));
    assert_eq!(names(&second), vec!["b", "a"], "{second:?}");
}

#[test]
fn a_blocked_preflight_explains_the_next_step() {
    let blocked = Preflight {
        unindexed_too_large: true,
        source_index: IndexStatus::NotIndexed,
        ..indexed(60_000)
    };
    let (state, _) = act(picked_state(), CompareAction::RunCheck);
    let (state, _) = update(state, checked_event(blocked));
    let texts = assert_translated(&workspace(state), "blocked");
    assert!(has(
        &texts,
        "Create an index on the key fields, then check again"
    ));
}

#[test]
fn a_check_in_flight_is_shown_on_the_button() {
    let (state, _) = act(picked_state(), CompareAction::RunCheck);
    let texts = assert_translated(&workspace(state), "checking");
    let shown = texts.iter().filter(|t| t.contains("Checking...")).count();
    assert_eq!(
        shown, 1,
        "the button says it; the status line does not repeat it"
    );
}

#[test]
fn a_running_comparison_shows_its_counters() {
    let progress = ev(CompareEvent::Progress {
        session: 1,
        run: 1,
        pair: 1,
        scanned_source: 1234,
        scanned_target: 567,
        differences: 89,
    });
    let (state, _) = update(running_state(), progress);
    let texts = assert_translated(&workspace(state), "running");
    assert!(has(&texts, "89 differences found so far"));
    assert!(has(&texts, "Scanned 1234 source and 567 target documents"));
}

#[test]
fn results_with_every_category_empty_say_the_collections_match() {
    let (state, _) = update(running_state(), pair_done(1, Summary::default()));
    let (state, _) = update(state, run_done());
    let texts = assert_translated(&workspace(state), "identical");
    assert!(has(&texts, "No differences"));
}

#[test]
fn results_with_rows_render_in_all_three_categories() {
    let state = workspace(with_rows());
    let texts = assert_translated(&state, "different");
    // The tab title wraps the (also wrapped) category name and the count.
    assert!(has(&texts, "Different» (250)"));
    assert!(has(&texts, "Only in source» (4)"));
    assert!(has(&texts, "Only in target» (0)"));
    let (state, _) = act(state, CompareAction::SelectCategory(Category::OnlyInSource));
    let (state, _) = update(state, rows_event(Category::OnlyInSource, 0, 3));
    assert_translated(&state, "only in source");
    let (state, _) = act(state, CompareAction::SelectCategory(Category::OnlyInTarget));
    let texts = assert_translated(&state, "only in target");
    assert!(has(&texts, "No documents in this category"));
}

#[test]
fn a_page_still_loading_shows_a_loading_line() {
    let texts = assert_translated(&workspace(with_differences()), "loading rows");
    assert!(has(&texts, "Loading the differences"));
}

fn detail_state() -> AppState {
    let key = KeyTuple(vec![Bson::Int32(1)]);
    let (state, _) = act(with_rows(), CompareAction::Select(key.clone()));
    let loading = assert_translated(&workspace_clone(&state), "detail loading");
    assert!(has(&loading, "Loading the document differences"));
    let changes = vec![
        FieldChange {
            path: "name".into(),
            kind: ChangeKind::Changed,
            source: Some(Bson::String("x".into())),
            target: Some(Bson::String("y".into())),
        },
        FieldChange {
            path: "extra".into(),
            kind: ChangeKind::Added,
            source: None,
            target: Some(Bson::Int32(1)),
        },
    ];
    let detail = CompareEvent::Detail {
        session: 1,
        pair: 1,
        key,
        source: Some(doc! {"_id": 1, "name": "x"}),
        target: Some(doc! {"_id": 1, "name": "y", "extra": 1}),
        changes,
    };
    update(state, ev(detail)).0
}

fn workspace_clone(state: &AppState) -> AppState {
    AppState {
        workspace: Workspace::Compare,
        compare: state.compare.clone(),
        conn: crate::views::connections::ConnectionsState::new(state.conn.connections.clone()),
        ..AppState::default()
    }
}

#[test]
fn an_open_detail_lists_the_changes_and_both_documents() {
    let state = detail_state();
    let texts = assert_translated(&workspace_clone(&state), "detail");
    assert!(has(&texts, "Changed"));
    assert!(has(&texts, "Added"));
    assert!(has(&texts, "Source document"));
    assert!(has(&texts, "Target document"));
    assert!(texts.iter().any(|t| t.contains("\"name\"")));
}

#[test]
fn a_document_missing_on_one_side_says_so() {
    let key = KeyTuple(vec![Bson::Int32(1)]);
    let (state, _) = act(with_rows(), CompareAction::Select(key.clone()));
    let only_source = CompareEvent::Detail {
        session: 1,
        pair: 1,
        key,
        source: Some(doc! {"_id": 1}),
        target: None,
        changes: Vec::new(),
    };
    let (state, _) = update(state, ev(only_source));
    let texts = assert_translated(&workspace_clone(&state), "one-sided detail");
    assert!(has(&texts, "does not exist on this side"));
    assert!(has(&texts, "no field differences"));
}

#[test]
fn a_failed_run_and_a_cancelled_run_render_their_outcome() {
    let failed = ev(CompareEvent::PairDone {
        session: 1,
        run: 1,
        pair: 1,
        outcome: PairOutcome::Failed(CoreError::CompareCapExceeded),
    });
    let (state, _) = update(running_state(), failed);
    let (state, _) = update(state, run_done());
    let texts = assert_translated(&workspace_clone(&state), "failed run");
    assert!(has(&texts, "did not finish"));
    let (state, _) = update(
        finished_with_results(),
        ev(CompareEvent::RunDone { session: 1, run: 1 }),
    );
    assert_translated(&workspace_clone(&state), "finished run");
}

#[test]
fn the_buttons_follow_the_view_model() {
    // Check is disabled while a check is in flight; Start only on a fresh Ok preflight.
    let (state, _) = act(picked_state(), CompareAction::RunCheck);
    assert!(state.compare.check_in_flight && !state.compare.can_check());
    assert!(checked_state().compare.can_start());
    assert!(!picked_state().compare.can_start());
    assert!(!running_state().compare.can_start());
}
