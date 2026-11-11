use super::*;
use bson::doc;
use curlybrace_core::shell::{DocKind, Outcome, ShellResult};

#[test]
fn a_short_text_is_kept_whole_on_one_line() {
    let i18n = I18n::english();
    let shown = shown_text("a\nb\tc", &i18n);
    assert_eq!(shown.shown, "a b c");
    assert_eq!(shown.hover, None);
}

#[test]
fn a_long_text_is_cut_by_characters_and_hovered_capped() {
    let i18n = I18n::english();
    let text = "é".repeat(1 << 20);
    let shown = shown_text(&text, &i18n);
    let cut = i18n.t("shell.result.cut");
    assert_eq!(shown.shown, format!("{}{cut}", "é".repeat(CELL_MAX_CHARS)));
    let hover = shown.hover.expect("a hover text");
    assert!(hover.starts_with(&"é".repeat(HOVER_MAX_CHARS)));
    assert!(hover.chars().count() < HOVER_MAX_CHARS + 60);
}

#[test]
fn a_text_between_the_caps_hovers_whole() {
    let i18n = I18n::english();
    let text = "y".repeat(CELL_MAX_CHARS + 1);
    assert_eq!(shown_text(&text, &i18n).hover, Some(text));
}

#[test]
fn a_long_key_is_cut_on_one_line() {
    let i18n = I18n::english();
    let key = format!("a\n{}", "k".repeat(1_000));
    let cut = shown_key(&key, &i18n);
    assert!(cut.starts_with("a k"));
    assert!(cut.chars().count() <= KEY_MAX_CHARS + 1);
    assert_eq!(shown_key("short", &i18n), "short");
}

const ONE: Uuid = Uuid::from_u128(1);
const TWO: Uuid = Uuid::from_u128(2);

/// A shell after `runs` clean runs, each returning `docs` documents (a new value each time).
fn ran(runs: u64, docs: i32) -> ShellState {
    let mut shell = ShellState::default().edited("db.a.find()".into(), None);
    for _ in 0..runs {
        let started = shell.run_requested().0;
        let run_id = started.result.run_id;
        let result = ShellResult::Documents {
            kind: DocKind::Cursor,
            docs: (0..docs).map(|i| doc! { "_id": i }).collect(),
            more: false,
            total: None,
        };
        shell = started
            .result_arrived(run_id, result)
            .finished(run_id, Outcome::Exited(0));
    }
    shell
}

/// Builds both the Table and the Tree of `shell` for `session`.
fn built(ctx: &egui::Context, session: Uuid, shell: &ShellState) {
    let i18n = I18n::english();
    assert!(table_cells(ctx, session, shell, &i18n).is_some());
    assert!(tree_lines(ctx, session, shell, &i18n).is_some());
}

fn has_entries(ctx: &egui::Context, session: Uuid) -> (bool, bool) {
    (
        cached_table(ctx, session).is_some(),
        cached_tree(ctx, session).is_some(),
    )
}

#[test]
fn forgetting_a_session_drops_both_entries() {
    let ctx = egui::Context::default();
    built(&ctx, ONE, &ran(1, 2));
    assert_eq!(has_entries(&ctx, ONE), (true, true));
    forget(&ctx, ONE);
    assert_eq!(has_entries(&ctx, ONE), (false, false));
    assert!(!known(&ctx).contains(&ONE));
}

#[test]
fn closed_sessions_are_forgotten_and_open_ones_kept() {
    let ctx = egui::Context::default();
    built(&ctx, ONE, &ran(1, 2));
    built(&ctx, TWO, &ran(1, 3));
    retain_sessions(&ctx, &[TWO]);
    assert_eq!(has_entries(&ctx, ONE), (false, false));
    assert_eq!(has_entries(&ctx, TWO), (true, true));
    retain_sessions(&ctx, &[]);
    assert_eq!(has_entries(&ctx, TWO), (false, false));
    assert!(known(&ctx).is_empty());
}

#[test]
fn two_sessions_have_their_own_entries() {
    let ctx = egui::Context::default();
    let i18n = I18n::english();
    let (one, two) = (ran(1, 2), ran(1, 5));
    let first = table_cells(&ctx, ONE, &one, &i18n).expect("a table");
    let second = table_cells(&ctx, TWO, &two, &i18n).expect("a table");
    assert_eq!((first.rows.len(), second.rows.len()), (2, 5));
    let again = table_cells(&ctx, ONE, &one, &i18n).expect("a table");
    assert!(
        Arc::ptr_eq(&first, &again),
        "session two replaced session one's rows"
    );
}

#[test]
fn a_second_run_is_built_once_and_replaces_the_first() {
    let ctx = egui::Context::default();
    let i18n = I18n::english();
    let first_run = ran(1, 2);
    let first = table_cells(&ctx, ONE, &first_run, &i18n).expect("a table");
    let second_run = ran(2, 4);
    let second = table_cells(&ctx, ONE, &second_run, &i18n).expect("a table");
    assert!(!Arc::ptr_eq(&first, &second));
    assert_eq!(second.rows.len(), 4);
    let cached = cached_table(&ctx, ONE).expect("cached");
    assert!(
        Arc::ptr_eq(&cached, &second),
        "the first run's rows are still kept"
    );
    let again = table_cells(&ctx, ONE, &second_run, &i18n).expect("a table");
    assert!(Arc::ptr_eq(&again, &second), "built twice");
}

#[test]
fn stale_entries_are_dropped_in_both_modes() {
    let ctx = egui::Context::default();
    let i18n = I18n::english();
    let first_run = ran(1, 2);
    built(&ctx, ONE, &first_run);
    drop_stale(&ctx, ONE, &first_run);
    assert_eq!(has_entries(&ctx, ONE), (true, true), "current rows kept");

    // A new run in flight has no value yet: both go.
    let pending = first_run.run_requested().0;
    drop_stale(&ctx, ONE, &pending);
    assert_eq!(has_entries(&ctx, ONE), (false, false));

    // The second run is viewed in the Table only: the Tree's old entry goes too.
    built(&ctx, ONE, &first_run);
    let second_run = ran(2, 3);
    assert!(table_cells(&ctx, ONE, &second_run, &i18n).is_some());
    drop_stale(&ctx, ONE, &second_run);
    assert_eq!(has_entries(&ctx, ONE), (true, false));
}
