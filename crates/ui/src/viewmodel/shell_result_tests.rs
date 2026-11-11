use super::*;
use crate::I18n;
use crate::viewmodel::shell::ShellNotice;
use crate::viewmodel::shell_result_table::{MAX_TABLE_COLUMNS, capped_columns, table_cell_text};
use crate::viewmodel::shell_result_tree::{MAX_EXPAND_ALL_ROWS, MAX_EXPANDED_PATHS};
use crate::viewmodel::tree_state::PageRow;
use bson::{Bson, Document, doc};
use curlybrace_core::runtime::ShellRefusal;
use curlybrace_core::shell::{DocKind, Outcome};

fn running() -> (ShellState, u64) {
    let (state, request) = ShellState::default()
        .edited("db.people.find()".into(), None)
        .run_requested();
    (state, request.expect("a request").run_id)
}

fn documents(docs: Vec<Document>) -> ShellResult {
    ShellResult::Documents {
        kind: DocKind::Cursor,
        docs,
        more: false,
        total: None,
    }
}

fn people() -> ShellResult {
    documents(vec![
        doc! { "_id": 1_i32, "name": "ada", "tags": ["x", "y"] },
        doc! { "_id": 2_i32, "name": "bob", "addr": { "city": "Oslo" } },
    ])
}

fn object() -> ShellResult {
    ShellResult::Object {
        type_name: None,
        doc: doc! { "ok": 1_i32, "n": { "a": 1_i32 } },
    }
}

fn values() -> ShellResult {
    ShellResult::Values {
        values: vec![Bson::Int32(1), Bson::Document(doc! { "a": 1_i32 })],
        more: false,
        total: 2,
    }
}

fn scalar(undefined: bool) -> ShellResult {
    ShellResult::Scalar {
        value: match undefined {
            true => Bson::Undefined,
            false => Bson::Int32(42),
        },
        undefined,
    }
}

/// A finished clean run whose last value was `result`.
fn ready(result: ShellResult) -> ShellState {
    let (state, id) = running();
    state
        .result_arrived(id, result)
        .finished(id, Outcome::Exited(0))
}

fn unavailable(reason: Reason) -> Availability {
    Availability::Unavailable(reason)
}

fn path(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

fn field_keys(rows: &[PageRow]) -> Vec<(usize, String)> {
    rows.iter()
        .filter_map(|r| match r {
            PageRow::Field { doc_index, row } => Some((*doc_index, row.path.join("."))),
            PageRow::Header { .. } => None,
        })
        .collect()
}

#[test]
fn defaults_are_text_mode_and_no_result() {
    let state = ShellState::default();
    assert_eq!(state.mode, OutputMode::Text);
    assert_eq!(state.result.state, ResultState::NoResult);
    assert_eq!(state.result.result, None);
    assert_eq!(
        state.availability(OutputMode::Text),
        Availability::Available
    );
    assert_eq!(
        state.availability(OutputMode::Table),
        unavailable(Reason::NoValue)
    );
}

#[test]
fn a_new_run_clears_the_previous_result() {
    let state = ready(people());
    assert_eq!(state.result.state, ResultState::Ready);

    let (state, request) = state.run_requested();
    let run_id = request.expect("a request").run_id;

    assert_eq!(state.result.run_id, run_id);
    assert_eq!(state.result.state, ResultState::Pending);
    assert_eq!(state.result.result, None);
    assert!(state.table_docs().is_empty());
    assert!(state.tree_rows().is_empty());
    assert_eq!(
        state.availability(OutputMode::Table),
        unavailable(Reason::Pending)
    );
}

#[test]
fn a_stale_result_is_ignored() {
    let (state, first) = running();
    let state = state.finished(first, Outcome::Exited(0));
    let (state, request) = state.run_requested();
    let second = request.expect("a request").run_id;

    let late = state.result_arrived(first, people());
    assert_eq!(late, state, "an older run's result changes nothing");

    let finished = state.finished(second, Outcome::Exited(0));
    assert_eq!(finished.result_arrived(second, people()), finished);
    assert_eq!(
        ShellState::default().result_arrived(1, people()),
        ShellState::default()
    );
}

#[test]
fn a_result_survives_its_runs_finish() {
    let state = ready(people());
    assert_eq!(state.result.state, ResultState::Ready);
    assert_eq!(
        state.availability(OutputMode::Table),
        Availability::Available
    );
}

#[test]
fn table_is_available_for_documents_and_an_object_only() {
    for result in [people(), object(), documents(Vec::new())] {
        assert_eq!(
            ready(result).availability(OutputMode::Table),
            Availability::Available
        );
    }
    for result in [values(), scalar(false)] {
        assert_eq!(
            ready(result).availability(OutputMode::Table),
            unavailable(Reason::NotDocuments)
        );
    }
}

#[test]
fn tree_is_available_for_values_too() {
    for result in [people(), object(), values()] {
        assert_eq!(
            ready(result).availability(OutputMode::Tree),
            Availability::Available
        );
    }
    assert_eq!(
        ready(scalar(false)).availability(OutputMode::Tree),
        unavailable(Reason::NotDocuments)
    );
}

#[test]
fn every_unavailable_reason_is_reported() {
    let too_big = ShellResult::TooBig { bytes: 5_000_000 };
    let unsupported = ShellResult::Unsupported {
        type_name: Some("Map".into()),
    };
    let (in_flight, _) = running();
    let (failing, id) = running();
    let failed = failing.finished(id, Outcome::ScriptErrors);
    let (empty, id) = running();
    let clean_without_value = empty.finished(id, Outcome::Exited(0));

    let cases = [
        (ready(scalar(false)), Reason::NotDocuments),
        (ready(scalar(true)), Reason::NoValue),
        (ready(too_big), Reason::TooBig(5_000_000)),
        (ready(unsupported), Reason::Unsupported),
        (failed, Reason::RunFailed),
        (in_flight, Reason::Pending),
        (clean_without_value, Reason::NoValue),
    ];
    for (state, reason) in cases {
        for mode in [OutputMode::Table, OutputMode::Tree] {
            assert_eq!(state.availability(mode), unavailable(reason), "{mode:?}");
        }
        assert_eq!(
            state.availability(OutputMode::Text),
            Availability::Available
        );
    }
    assert_eq!(
        ready(values()).availability(OutputMode::Table),
        unavailable(Reason::NotDocuments)
    );
}

/// The whole (result, mode) -> availability table in one place.
#[test]
fn the_availability_table() {
    use Availability::Available as A;
    let nd = unavailable(Reason::NotDocuments);
    let rows: [(ShellResult, [Availability; 3]); 8] = [
        (people(), [A, A, A]),
        (documents(Vec::new()), [A, A, A]),
        (object(), [A, A, A]),
        (values(), [A, nd, A]),
        (scalar(false), [A, nd, nd]),
        (
            scalar(true),
            [
                A,
                unavailable(Reason::NoValue),
                unavailable(Reason::NoValue),
            ],
        ),
        (
            ShellResult::TooBig { bytes: 9 },
            [
                A,
                unavailable(Reason::TooBig(9)),
                unavailable(Reason::TooBig(9)),
            ],
        ),
        (
            ShellResult::Unsupported { type_name: None },
            [
                A,
                unavailable(Reason::Unsupported),
                unavailable(Reason::Unsupported),
            ],
        ),
    ];
    let modes = [OutputMode::Text, OutputMode::Table, OutputMode::Tree];
    for (result, expected) in rows {
        let label = format!("{result:?}");
        let state = ready(result);
        for (mode, want) in modes.iter().zip(expected) {
            assert_eq!(state.availability(*mode), want, "{label} in {mode:?}");
        }
    }
}

#[test]
fn the_mode_survives_a_new_run() {
    let state = ready(people()).mode_set(OutputMode::Table);
    assert_eq!(state.mode, OutputMode::Table);
    let (state, _) = state.run_requested();
    assert_eq!(state.mode, OutputMode::Table);
    assert_eq!(state.mode_set(OutputMode::Tree).mode, OutputMode::Tree);
}

#[test]
fn a_finished_run_without_a_result_is_no_result() {
    for (outcome, reason) in [
        (Outcome::Exited(0), Reason::NoValue),
        (Outcome::Exited(1), Reason::RunFailed),
        (Outcome::Cancelled, Reason::RunFailed),
        (Outcome::Incomplete, Reason::RunFailed),
    ] {
        let (state, id) = running();
        let state = state.finished(id, outcome);
        assert_eq!(state.result.state, ResultState::NoResult, "{outcome:?}");
        assert_eq!(state.result.result, None);
        assert_eq!(
            state.availability(OutputMode::Tree),
            unavailable(reason),
            "{outcome:?}"
        );
    }
}

#[test]
fn a_refused_run_has_no_result() {
    let (state, id) = running();
    let state = state.refused(id, ShellRefusal::AlreadyRunning);
    assert_eq!(state.result.state, ResultState::NoResult);
    assert_eq!(
        state.availability(OutputMode::Table),
        unavailable(Reason::RunFailed)
    );
}

#[test]
fn an_empty_script_refusal_keeps_the_previous_result() {
    let state = ready(people()).mode_set(OutputMode::Table);
    let (state, request) = state.edited("   ".into(), None).run_requested();
    assert_eq!(request, None);
    assert_eq!(state.notice, Some(ShellNotice::EmptyScript));
    assert_eq!(state.result.state, ResultState::Ready);
    assert_eq!(
        state.availability(OutputMode::Table),
        Availability::Available
    );
}

#[test]
fn an_interrupted_run_has_failed_and_an_idle_shell_keeps_its_value() {
    let idle = ready(people()).mode_set(OutputMode::Tree);
    assert_eq!(idle.result_interrupted(), idle, "nothing was running");
    assert_eq!(
        ShellState::default().result_interrupted(),
        ShellState::default()
    );

    let (in_flight, _) = running();
    let cut = in_flight.mode_set(OutputMode::Tree).result_interrupted();
    assert_eq!(cut.result.state, ResultState::NoResult);
    assert_eq!(cut.mode, OutputMode::Tree);
    assert_eq!(
        cut.availability(OutputMode::Tree),
        unavailable(Reason::RunFailed)
    );

    // The value came (it is sent right before the run's end), then the connection went.
    let (arrived, id) = running();
    let arrived = arrived.result_arrived(id, people());
    assert_eq!(arrived.result_interrupted().result, arrived.result);
}

#[test]
fn columns_are_the_union_in_first_seen_order_capped_at_50_with_hidden_count() {
    let narrow = capped_columns(&[
        doc! { "b": 1_i32, "_id": 1_i32 },
        doc! { "c": 1_i32, "b": 2_i32 },
    ]);
    assert_eq!(narrow.shown, ["_id", "b", "c"]);
    assert_eq!(narrow.hidden_columns, 0);

    let first: Document = (0..40).map(|i| (format!("a{i}"), Bson::Int32(i))).collect();
    let second: Document = (0..30).map(|i| (format!("b{i}"), Bson::Int32(i))).collect();
    let wide = capped_columns(&[first, second]);
    assert_eq!(wide.shown.len(), MAX_TABLE_COLUMNS);
    assert_eq!(wide.hidden_columns, 70 - MAX_TABLE_COLUMNS);
    assert_eq!(wide.shown[0], "a0");
    assert_eq!(wide.shown[40], "b0");
    assert_eq!(wide.shown[49], "b9");

    let state = ready(people());
    assert_eq!(
        state.result_columns().shown,
        ["_id", "name", "tags", "addr"]
    );
    assert_eq!(state.table_docs().len(), 2);
}

#[test]
fn an_object_is_one_row() {
    let state = ready(object());
    assert_eq!(
        state.table_docs(),
        [doc! { "ok": 1_i32, "n": { "a": 1_i32 } }]
    );
    assert_eq!(state.result_columns().shown, ["ok", "n"]);
    let i18n = I18n::english();
    let row = &state.table_docs()[0];
    assert_eq!(table_cell_text(row, "ok", &i18n), Some("1".to_string()));
    assert_eq!(table_cell_text(row, "missing", &i18n), None);
}

#[test]
fn no_table_rows_where_the_table_is_unavailable() {
    for result in [values(), scalar(false), ShellResult::TooBig { bytes: 1 }] {
        let state = ready(result);
        assert!(state.table_docs().is_empty());
        assert!(state.result_columns().shown.is_empty());
    }
}

#[test]
fn values_show_in_the_tree_as_one_document_keyed_by_index() {
    let state = ready(values());
    let rows = state.tree_rows();
    assert_eq!(rows[0], PageRow::Header { doc_index: 0 });
    assert_eq!(field_keys(&rows), [(0, "0".into()), (0, "1".into())]);
    let open = state.tree_toggled(0, path(&["1"]));
    assert_eq!(
        field_keys(&open.tree_rows()),
        [(0, "0".into()), (0, "1".into()), (0, "1.a".into())]
    );
}

#[test]
fn tree_toggle_and_expand_all_and_collapse_all() {
    let state = ready(people());
    assert_eq!(field_keys(&state.tree_rows()).len(), 6);

    let toggled = state.tree_toggled(1, path(&["addr"]));
    assert!(toggled.result.tree.is_expanded(1, &path(&["addr"])));
    assert!(!toggled.result.tree.is_expanded(0, &path(&["addr"])));
    assert!(field_keys(&toggled.tree_rows()).contains(&(1, "addr.city".into())));
    let back = toggled.tree_toggled(1, path(&["addr"]));
    assert!(!back.result.tree.is_expanded(1, &path(&["addr"])));
    assert!(
        !state.result.tree.is_expanded(1, &path(&["addr"])),
        "the original is not changed"
    );

    let all = state.tree_all_set(true);
    assert!(all.result.tree.is_expanded(0, &path(&["tags"])));
    assert!(all.result.tree.is_expanded(1, &path(&["addr"])));
    assert!(!all.result.tree.capped);
    assert_eq!(field_keys(&all.tree_rows()).len(), 6 + 2 + 1);

    let none = all.tree_all_set(false);
    assert_eq!(field_keys(&none.tree_rows()).len(), 6);
    assert!(!none.result.tree.is_expanded(0, &path(&["tags"])));
}

#[test]
fn tree_changes_are_ignored_without_a_tree() {
    let state = ready(scalar(false));
    assert_eq!(state.tree_toggled(0, path(&["a"])), state);
    assert_eq!(state.tree_all_set(true), state);
    let docs = ready(people());
    assert_eq!(
        docs.tree_toggled(7, path(&["addr"])),
        docs,
        "no such document"
    );
}

#[test]
fn expand_all_is_capped() {
    // Every document holds many small sub-documents: far more expandable paths than the cap.
    let doc_of = |i: i32| -> Document {
        (0..100)
            .map(|k| (format!("k{k}"), Bson::Document(doc! { "v": i })))
            .collect()
    };
    let state = ready(documents((0..200).map(doc_of).collect()));
    let all = state.tree_all_set(true);
    let open: usize = all.result.tree.len();
    assert!(all.result.tree.capped);
    assert!(open <= MAX_EXPANDED_PATHS, "{open}");
    assert!(open > 0);

    // Wide arrays: the revealed-rows budget stops it before the paths cap.
    let wide = |_: i32| -> Document {
        let arr: Vec<Bson> = (0..200).map(Bson::Int32).collect();
        (0..20)
            .map(|k| (format!("a{k}"), Bson::Array(arr.clone())))
            .collect()
    };
    let rows = ready(documents((0..200).map(wide).collect())).tree_all_set(true);
    assert!(rows.result.tree.capped);
    assert!(rows.result.tree.len() * 200 <= MAX_EXPAND_ALL_ROWS);

    let collapsed = all.tree_all_set(false);
    assert!(!collapsed.result.tree.capped);
    assert_eq!(collapsed.result.tree.len(), 0);
}

#[test]
fn the_tree_resets_for_a_new_result() {
    let state = ready(people()).tree_all_set(true);
    assert!(!state.result.tree.is_empty());
    let (state, request) = state.run_requested();
    assert_eq!(state.result.tree.len(), 0);
    let id = request.expect("a request").run_id;
    let state = state.result_arrived(id, people());
    assert_eq!(state.result.tree.len(), 0);
    assert!(!state.result.tree.capped);
    assert_eq!(field_keys(&state.tree_rows()).len(), 6);
}
