//! Text boxes expose their caption as their accessible name, which is what screen readers and
//! the click-driven UI tests use to find them. Fast, offline, no GPU.
use curlybrace_ui::I18n;
use curlybrace_ui::viewmodel::{EditorState, FindBarState};
use curlybrace_ui::views::connections::{ConnectionsState, show_connections};
use curlybrace_ui::views::editor::show_editor;
use curlybrace_ui::views::findbar::show_findbar;
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

fn size() -> egui::Vec2 {
    egui::vec2(900.0, 700.0)
}

#[test]
fn connection_form_boxes_are_found_by_their_caption() {
    let i18n = I18n::english();
    let mut harness = Harness::builder().with_size(size()).build_ui_state(
        |ui, state: &mut ConnectionsState| {
            show_connections(ui, state, &I18n::english());
        },
        ConnectionsState::default(),
    );
    harness.run();
    for key in [
        "connections.name",
        "connections.uri",
        "connections.username",
        "connections.auth_source",
        "connections.password",
    ] {
        let label = i18n.t(key);
        let role = if key == "connections.password" {
            Role::PasswordInput
        } else {
            Role::TextInput
        };
        harness.get_by_role_and_label(role, &label);
    }
}

#[test]
fn find_bar_boxes_are_found_by_their_caption_and_keep_typed_text() {
    let i18n = I18n::english();
    let mut harness = Harness::builder().with_size(size()).build_ui_state(
        |ui, state: &mut FindBarState| {
            show_findbar(ui, state, false, false, &I18n::english());
        },
        FindBarState::default(),
    );
    harness.run();
    let filter = i18n.t("findbar.filter");
    harness.get_by_label(&filter).click();
    harness.run();
    harness.get_by_label(&filter).type_text("{ \"a\": 1 }");
    harness.run();
    assert_eq!(harness.state().text.filter, "{ \"a\": 1 }");
    for key in [
        "findbar.sort",
        "findbar.projection",
        "findbar.skip",
        "findbar.limit",
    ] {
        harness.get_by_label(&i18n.t(key));
    }
}

mod compare_workspace {
    use super::*;
    use bson::{Bson, doc};
    use curlybrace_core::compare::diff::{ChangeKind, FieldChange, PathSegment};
    use curlybrace_core::compare::engine::Summary;
    use curlybrace_core::compare::key::KeyTuple;
    use curlybrace_core::compare::protocol::{
        CompareEvent, DuplicateReport, PairOutcome, Preflight, RowView, Side,
    };
    use curlybrace_core::compare::results::Category;
    use curlybrace_core::connection::ConnectionConfig;
    use curlybrace_core::error::CoreError;
    use curlybrace_core::runtime::Event;
    use curlybrace_core::store::{CollectionKind, DuplicateGroup, IndexStatus};
    use curlybrace_ui::app::compare::CompareAction;
    use curlybrace_ui::app::reducer::{Input, update};
    use curlybrace_ui::app::state::AppState;
    use curlybrace_ui::viewmodel::compare::{CompareState, Pick};
    use curlybrace_ui::views::compare::show_compare;
    use egui_kittest::kittest::NodeT;

    type Fixture = (CompareState, ConnectionsState);

    fn step(state: AppState, input: Input) -> AppState {
        update(state, input).0
    }

    fn event(e: CompareEvent) -> Input {
        Input::Event(Event::Compare(e))
    }

    fn act(state: AppState, a: CompareAction) -> AppState {
        step(state, Input::Compare(a))
    }

    fn key(n: i32) -> KeyTuple {
        KeyTuple(vec![Bson::Int32(n)])
    }

    fn picked(state: AppState, side: Side) -> AppState {
        let state = act(
            state,
            CompareAction::PickSide(side, Pick::Database("shop".into())),
        );
        let listing = CompareEvent::Collections {
            session: 1,
            side,
            db: "shop".into(),
            entries: vec![("users".into(), CollectionKind::Collection)],
        };
        let state = step(state, event(listing));
        act(
            state,
            CompareAction::PickSide(side, Pick::Collection("users".into())),
        )
    }

    /// A session with a ready form, finished results with rows and an open detail.
    fn fixture(through: &str) -> Fixture {
        let (a, b) = (
            ConnectionConfig::new("one", "mongodb://a"),
            ConnectionConfig::new("two", "mongodb://b"),
        );
        let open = CompareAction::Open {
            source: a.id,
            target: b.id,
        };
        let state = act(AppState::new(vec![a, b]), open);
        let state = step(state, event(CompareEvent::Opened { session: 1 }));
        let state = picked(picked(state, Side::Source), Side::Target);
        let state = match through {
            "picked" => state,
            _ => act(state, CompareAction::AddPair),
        };
        let state = match through {
            "form" | "picked" => state,
            "checking" => act(state, CompareAction::RunCheck),
            // Checked fine, then edited: the answer is gone and the preflight is Unchecked.
            "edited" => act(checked(state), CompareAction::SetKey("sku".into())),
            // Edited while a check was in flight: its answer will be dropped.
            "stale" => {
                let state = act(state, CompareAction::RunCheck);
                act(state, CompareAction::SetKey("sku".into()))
            }
            "checked" => checked(state),
            "duplicates" => duplicate_failure(state),
            "duplicates_loading" => act(duplicate_failure(state), CompareAction::FindDuplicates(0)),
            "duplicates_loaded" => duplicates_loaded(duplicate_failure(state)),
            _ => finished(state),
        };
        (state.compare, state.conn)
    }

    fn checked(state: AppState) -> AppState {
        let state = act(state, CompareAction::RunCheck);
        let preflight = Preflight {
            source_count: 3,
            target_count: 3,
            source_index: IndexStatus::Indexed,
            target_index: IndexStatus::Indexed,
            unindexed_too_large: false,
        };
        let answer = CompareEvent::Checked {
            session: 1,
            pair: 1,
            preflight,
        };
        step(state, event(answer))
    }

    /// Checked, started, and the run ended on a duplicate key.
    fn duplicate_failure(state: AppState) -> AppState {
        let state = act(checked(state), CompareAction::Start);
        let failed = CompareEvent::PairDone {
            session: 1,
            run: 1,
            pair: 2, // the run follows the check, so it has the next pair id
            outcome: PairOutcome::Failed(CoreError::CompareDuplicateKey),
        };
        let state = step(state, event(failed));
        step(state, event(CompareEvent::RunDone { session: 1, run: 1 }))
    }

    /// Find was clicked and both sides answered: one group on the source, none on the target.
    fn duplicates_loaded(state: AppState) -> AppState {
        let state = act(state, CompareAction::FindDuplicates(0));
        let group = DuplicateGroup {
            key: KeyTuple(vec![Bson::String("dup@example.com".into())]),
            count: 2,
        };
        let answer = CompareEvent::DuplicateKeys {
            session: 1,
            pair: 3, // a lookup takes a fresh id: after the check (1) and the run (2)
            source: DuplicateReport {
                groups: vec![group],
                more: true,
            },
            target: DuplicateReport {
                groups: vec![],
                more: false,
            },
        };
        step(state, event(answer))
    }

    fn button_disabled(harness: &Harness<'_, Fixture>, label: &str) -> bool {
        let button = harness.get_by_role_and_label(Role::Button, label);
        button.accesskit_node().is_disabled()
    }

    #[test]
    fn add_pair_is_its_own_button_and_check_never_adds_the_picked_pair() {
        let i18n = I18n::english();
        let mut picked = harness(fixture("picked"));
        picked.run();
        assert!(!button_disabled(&picked, &i18n.t("compare.pairs.add_pair")));
        assert!(
            button_disabled(&picked, &i18n.t("compare.preflight.check")),
            "nothing in the list to check"
        );

        // Clicking it reports AddPair (and only that).
        let (state, connections) = fixture("picked");
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1100.0, 1400.0))
            .build_ui_state(
                |ui, (s, c, out): &mut (CompareState, ConnectionsState, Option<CompareAction>)| {
                    *out = show_compare(ui, s, c, &I18n::english()).or(out.take());
                },
                (state, connections, None),
            );
        harness.run();
        harness
            .get_by_label(&i18n.t("compare.pairs.add_pair"))
            .click();
        harness.run();
        assert_eq!(harness.state().2, Some(CompareAction::AddPair));
    }

    #[test]
    fn check_is_disabled_while_a_check_is_in_flight_and_start_follows_the_view_model() {
        let i18n = I18n::english();
        let check = i18n.t("compare.preflight.check");
        let start = i18n.t("compare.preflight.run");
        let checking = i18n.t("compare.preflight.checking");

        let mut ready = harness(fixture("form"));
        ready.run();
        assert!(
            !button_disabled(&ready, &check),
            "a ready form can be checked"
        );
        assert!(button_disabled(&ready, &start), "unchecked: no start");

        // The spinner repaints continuously, so step instead of running to rest.
        let mut in_flight = harness(fixture("checking"));
        in_flight.run_steps(2);
        assert!(button_disabled(&in_flight, &checking), "check is in flight");
        assert!(button_disabled(&in_flight, &start));

        let mut stale = harness(fixture("stale"));
        stale.run_steps(2);
        assert!(button_disabled(&stale, &checking));
        assert!(button_disabled(&stale, &start), "stale answer pending");

        let mut edited = harness(fixture("edited"));
        edited.run();
        assert!(!button_disabled(&edited, &check));
        assert!(button_disabled(&edited, &start), "edit after a good check");

        let mut fresh = harness(fixture("checked"));
        fresh.run();
        assert!(!button_disabled(&fresh, &start), "fresh Ok answer starts");
    }

    #[test]
    fn connect_is_disabled_for_the_open_pair_and_close_is_enabled() {
        let i18n = I18n::english();
        let mut open = harness(fixture("form"));
        open.run();
        assert!(button_disabled(&open, &i18n.t("compare.session.open")));
        assert!(!button_disabled(&open, &i18n.t("compare.session.close")));
    }

    fn finished(state: AppState) -> AppState {
        let state = act(state, CompareAction::RunCheck);
        let preflight = Preflight {
            source_count: 3,
            target_count: 3,
            source_index: IndexStatus::Indexed,
            target_index: IndexStatus::Indexed,
            unindexed_too_large: false,
        };
        let checked = CompareEvent::Checked {
            session: 1,
            pair: 1,
            preflight,
        };
        let state = act(step(state, event(checked)), CompareAction::Start);
        let summary = Summary {
            different: 2,
            only_in_source: 1,
            ..Summary::default()
        };
        let done = CompareEvent::PairDone {
            session: 1,
            run: 1,
            pair: 2, // the run follows the check, so it has the next pair id
            outcome: PairOutcome::Completed(summary),
        };
        let state = step(state, event(done));
        let state = step(state, event(CompareEvent::RunDone { session: 1, run: 1 }));
        let rows = CompareEvent::Rows {
            session: 1,
            pair: 2, // the run follows the check, so it has the next pair id
            category: Category::Different,
            offset: 0,
            total: 2,
            rows: (1..=2)
                .map(|n| RowView {
                    key: key(n),
                    has_source: true,
                    has_target: true,
                })
                .collect(),
        };
        let state = step(state, event(rows));
        let state = act(state, CompareAction::Select(key(1)));
        let change = FieldChange {
            path: "name".into(),
            segments: vec![PathSegment::Field("name".into())],
            kind: ChangeKind::Changed,
            source: Some(Bson::String("x".into())),
            target: Some(Bson::String("y".into())),
        };
        let detail = CompareEvent::Detail {
            session: 1,
            pair: 2, // the run follows the check, so it has the next pair id
            key: key(1),
            source: Some(doc! {"_id": 1, "name": "x"}),
            target: Some(doc! {"_id": 1, "name": "y"}),
            changes: vec![change],
        };
        step(state, event(detail))
    }

    fn harness(fixture: Fixture) -> Harness<'static, Fixture> {
        Harness::builder()
            .with_size(egui::vec2(1100.0, 1400.0))
            .build_ui_state(
                |ui, (state, connections): &mut Fixture| {
                    show_compare(ui, state, connections, &I18n::english());
                },
                fixture,
            )
    }

    const INTERACTIVE: [Role; 7] = [
        Role::Button,
        Role::TextInput,
        Role::MultilineTextInput,
        Role::CheckBox,
        Role::ComboBox,
        Role::Link,
        Role::RadioButton,
    ];

    /// Every interactive node carries its own label or is labelled by another node.
    fn unlabelled(harness: &Harness<'_, Fixture>) -> (usize, Vec<String>) {
        let mut interactive = 0;
        let mut bad = Vec::new();
        for node in harness.root().children_recursive() {
            let ak = node.accesskit_node();
            if !INTERACTIVE.contains(&ak.role()) {
                continue;
            }
            interactive += 1;
            let named = |text: Option<&str>| text.is_some_and(|t| !t.trim().is_empty());
            let by_other = ak
                .labelled_by()
                .any(|l| named(l.label().as_deref()) || named(l.value().as_deref()));
            if !(named(ak.label().as_deref()) || by_other) {
                bad.push(format!("{:?}", ak.role()));
            }
        }
        (interactive, bad)
    }

    #[test]
    fn the_setup_form_has_named_controls_including_the_rules() {
        let mut harness = harness(fixture("form"));
        harness.run();
        harness
            .get_by_label(&I18n::english().t("compare.rules.title"))
            .click();
        harness.run();
        let (interactive, bad) = unlabelled(&harness);
        assert!(interactive >= 12, "only {interactive} interactive nodes");
        assert!(bad.is_empty(), "unlabelled controls: {bad:?}");
        let i18n = I18n::english();
        for key in [
            "compare.setup.key",
            "compare.setup.filter",
            "compare.rules.ignore_array_order",
            "compare.rules.scope",
            "compare.preflight.check",
            "compare.preflight.run",
        ] {
            harness.get_all_by_label(&i18n.t(key)).next().expect(key);
        }
    }

    #[test]
    fn results_and_detail_controls_are_named() {
        let mut harness = harness(fixture("detail"));
        harness.run();
        let (interactive, bad) = unlabelled(&harness);
        // The results screen replaces the setup: tabs, pager, rows, detail and Back.
        assert!(interactive >= 10, "only {interactive} interactive nodes");
        assert!(bad.is_empty(), "unlabelled controls: {bad:?}");
        let i18n = I18n::english();
        for key in [
            "compare.results.back_to_overview",
            "compare.results.prev",
            "compare.results.next",
            "compare.detail.close",
        ] {
            harness.get_by_label(&i18n.t(key));
        }
    }

    #[test]
    fn the_duplicate_failure_panel_has_named_controls() {
        let i18n = I18n::english();
        let find = i18n.t("compare.duplicates.find");
        let mut offered = harness(fixture("duplicates"));
        offered.run();
        assert!(
            !button_disabled(&offered, &find),
            "offered after the failure"
        );
        let (interactive, bad) = unlabelled(&offered);
        assert!(interactive >= 12, "only {interactive} interactive nodes");
        assert!(bad.is_empty(), "unlabelled controls: {bad:?}");

        let mut loading = harness(fixture("duplicates_loading"));
        loading.run_steps(2);
        assert!(button_disabled(&loading, &find), "disabled while loading");

        let mut loaded = harness(fixture("duplicates_loaded"));
        loaded.run();
        let (_, bad) = unlabelled(&loaded);
        assert!(bad.is_empty(), "unlabelled controls: {bad:?}");
        for key in [
            "compare.duplicates.source",
            "compare.duplicates.target",
            "compare.duplicates.more",
            "compare.duplicates.none_on_side",
        ] {
            let label = i18n.t(key);
            loaded.get_all_by_label(&label).next().expect(key);
        }
        let row = i18n.t_with(
            "compare.duplicates.row",
            &[("key", "\"dup@example.com\""), ("count", "2")],
        );
        loaded.get_by_label(&row);
    }

    #[test]
    fn clicking_a_key_row_asks_for_its_detail() {
        // The row is found by its text and reported through the returned action.
        let (state, connections) = fixture("detail");
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1100.0, 1400.0))
            .build_ui_state(
                |ui, (s, c, out): &mut (CompareState, ConnectionsState, Option<CompareAction>)| {
                    *out = show_compare(ui, s, c, &I18n::english()).or(out.take());
                },
                (state, connections, None),
            );
        harness.run();
        // The row's checkbox carries the same name; the row itself is the other node.
        harness
            .get_all_by_label("2")
            .find(|n| n.accesskit_node().role() != Role::CheckBox)
            .expect("the row")
            .click();
        harness.run();
        assert_eq!(harness.state().2, Some(CompareAction::Select(key(2))));
    }
}

#[test]
fn editor_text_box_is_named_by_the_dialog_title() {
    let i18n = I18n::english();
    let mut harness = Harness::builder().with_size(size()).build_state(
        |ctx, state: &mut EditorState| {
            show_editor(ctx, state, &I18n::english());
        },
        EditorState::for_new(),
    );
    harness.run();
    let title = i18n.t("editor.title_new");
    harness.get_by_role_and_label(Role::MultilineTextInput, &title);
}
