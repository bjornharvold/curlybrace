//! Headless frames of the pair table, the editor and the overview with many pairs: only the
//! visible rows are drawn, markers carry an icon and a label, and nothing is untranslated.
use super::frames::{assert_translated, has, workspace};
use super::*;
use crate::viewmodel::compare::PairOverrides;

/// `n` collections `c0..` on both sides of databases `a` and `b`.
fn many_state(n: usize) -> AppState {
    let (mut state, _, _) = opened();
    let entries: Vec<(String, CollectionKind)> = (0..n)
        .map(|i| (format!("c{i}"), CollectionKind::Collection))
        .collect();
    for (side, db) in [(Side::Source, "a"), (Side::Target, "b")] {
        let (s, _) = act(
            state,
            CompareAction::PickSide(side, Pick::Database(db.into())),
        );
        let listing = CompareEvent::Collections {
            session: 1,
            side,
            db: db.into(),
            entries: entries.clone(),
        };
        state = update(s, ev(listing)).0;
    }
    act(state, CompareAction::AddAllSameNamed).0
}

#[test]
fn hundreds_of_pairs_draw_only_the_visible_rows() {
    let state = workspace(many_state(400));
    assert_eq!(state.compare.pairs.entries.len(), 400);
    let texts = assert_translated(&state, "400 pairs");
    let rows = texts.iter().filter(|t| t.starts_with("a.c")).count();
    assert!(rows > 0, "the first rows are drawn");
    assert!(rows < 60, "{rows} source cells drawn: not virtualised");
    assert!(has(&texts, "a.c0"));
    assert!(!has(&texts, "a.c399"), "a row far below is not drawn");
}

#[test]
fn many_pairs_leave_add_all_with_nothing_to_add() {
    let state = many_state(30);
    let list = crate::viewmodel::compare::ListView::of(&state.compare);
    assert!(!list.can_add_all());
    assert!(list.unmatched.is_empty());
}

#[test]
fn a_customised_pair_is_marked_with_an_icon_and_a_label() {
    let state = many_state(3);
    let overrides = PairOverrides {
        key_text: Some("sku".into()),
        ..PairOverrides::default()
    };
    let (state, _) = act(state, CompareAction::SetPairOverrides(1, overrides));
    let texts = assert_translated(&workspace(state), "customised");
    let marked = texts.iter().filter(|t| *t == "≠ «Customised»").count();
    assert_eq!(marked, 1, "only the overridden pair: {texts:?}");
    assert!(has(&texts, "sku"), "its own key is shown");
}

#[test]
fn the_editor_names_the_pair_and_offers_use_default() {
    let (state, _) = act(many_state(2), CompareAction::EditPair(Some(1)));
    let texts = assert_translated(&workspace(state), "editor");
    assert!(has(&texts, "Settings for «a.c1 to b.c1»"), "{texts:?}");
    assert!(has(&texts, "Use default"));
    assert!(has(&texts, "Source filter (JSON)"));
    assert!(has(&texts, "Target filter (JSON)"));
    assert!(has(&texts, "Done"));
}

#[test]
fn collections_without_a_counterpart_are_listed_with_the_hint() {
    let (mut state, _, _) = opened();
    for (side, db, name) in [(Side::Source, "a", "only_here"), (Side::Target, "b", "x")] {
        let (s, _) = act(
            state,
            CompareAction::PickSide(side, Pick::Database(db.into())),
        );
        let listing = CompareEvent::Collections {
            session: 1,
            side,
            db: db.into(),
            entries: vec![(name.into(), CollectionKind::Collection)],
        };
        state = update(s, ev(listing)).0;
    }
    let texts = assert_translated(&workspace(state), "no counterpart");
    assert!(has(&texts, "No counterpart"));
    assert!(has(
        &texts,
        "Indexes and collection options are not compared"
    ));
    assert!(has(&texts, "only_here"));
    assert!(has(&texts, "Pair with this name"));
}

#[test]
fn a_started_run_lists_every_pair_waiting_with_icon_label_totals_and_cancel() {
    let (state, _) = act(many_state(3), CompareAction::RunCheck);
    // The queue sends the next check by itself after each answer: probes 1, 2 and 3.
    let state = (1..=3).fold(state, |state, probe| {
        let answer = CompareEvent::Checked {
            session: 1,
            pair: probe,
            preflight: indexed(5),
        };
        update(state, ev(answer)).0
    });
    let (state, _) = act(state, CompareAction::Start);
    assert!(state.compare.is_running());
    let texts = assert_translated(&workspace(state), "pending rows");
    let waiting = texts.iter().filter(|t| *t == "⏳ «Waiting»").count();
    assert_eq!(waiting, 3, "{texts:?}");
    assert!(has(&texts, "Totals"));
    assert!(has(&texts, "Cancel"));
}
