//! Copying single fields: the checkboxes of the detail pane, the two queue buttons with their
//! reasons, the pending line and the review dialog of a field copy. Fast, offline, no GPU.
mod support;

mod compare_fields {
    use super::support::compare_apply_states::{
        PAIR, act, applied, applying, check_out, finished, key, rows_of, run_active, step,
    };
    use super::support::compare_field_states::{
        ANY, CITY, DOLLAR, DOTTED, NAME, path_of, reviewing_fields, ticked, with_field_detail,
        with_fields_and_delete_pending, with_fields_pending,
    };
    use bson::Bson;
    use curlybrace_core::compare::key::KeyTuple;
    use curlybrace_core::compare::protocol::{CompareEvent, RowView};
    use curlybrace_core::compare::results::Category;
    use curlybrace_core::runtime::Event;
    use curlybrace_ui::I18n;
    use curlybrace_ui::app::compare::CompareAction;
    use curlybrace_ui::app::reducer::Input;
    use curlybrace_ui::app::state::AppState;
    use curlybrace_ui::viewmodel::compare::{FieldSelection, RowAction, key_label};
    use curlybrace_ui::views::compare::show_compare;
    use eframe::egui;
    use eframe::egui::accesskit::{Role, Toggled};
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    type Gate = fn(AppState) -> AppState;

    const TO_T: RowAction = RowAction::CopyFieldsToTarget;
    const TO_S: RowAction = RowAction::CopyFieldsToSource;

    /// The app (actions are fed back through the reducer, as the real app does) and every action
    /// the views emitted.
    struct Run {
        app: AppState,
        actions: Vec<CompareAction>,
    }

    fn harness(state: AppState) -> Harness<'static, Run> {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1100.0, 1800.0))
            .build_ui_state(
                |ui, run: &mut Run| {
                    let shown = show_compare(ui, &run.app.compare, &run.app.conn, &I18n::english());
                    if let Some(action) = shown {
                        run.actions.push(action.clone());
                        run.app = act(
                            std::mem::replace(&mut run.app, AppState::new(Vec::new())),
                            action,
                        );
                    }
                },
                Run {
                    app: state,
                    actions: Vec::new(),
                },
            );
        harness.run_steps(5);
        harness
    }

    fn t(key: &str) -> String {
        I18n::english().t(key)
    }

    fn tw(key: &str, params: &[(&str, &str)]) -> String {
        I18n::english().t_with(key, params)
    }

    fn actions(harness: &Harness<'_, Run>) -> Vec<CompareAction> {
        harness.state().actions.clone()
    }

    fn checkbox_disabled(harness: &Harness<'_, Run>, path: &str) -> bool {
        harness
            .get_by_role_and_label(Role::CheckBox, path)
            .accesskit_node()
            .is_disabled()
    }

    fn button_disabled(harness: &Harness<'_, Run>, label: &str) -> bool {
        harness
            .get_by_role_and_label(Role::Button, label)
            .accesskit_node()
            .is_disabled()
    }

    fn click(harness: &mut Harness<'_, Run>, label: &str) {
        harness.get_by_role_and_label(Role::Button, label).click();
        harness.run_steps(5);
    }

    fn to_target() -> String {
        t("compare.detail.copy_fields_to_target")
    }

    fn to_source() -> String {
        t("compare.detail.copy_fields_to_source")
    }

    // ---- checkboxes ----

    #[test]
    fn every_changed_field_has_a_checkbox_named_by_its_path() {
        let harness = harness(with_field_detail());
        for path in [NAME, CITY, DOLLAR, DOTTED, ANY] {
            harness.get_by_role_and_label(Role::CheckBox, path);
        }
    }

    #[test]
    fn clicking_a_field_checkbox_emits_toggle_field_with_its_segments() {
        let mut h = harness(with_field_detail());
        h.get_by_role_and_label(Role::CheckBox, CITY).click();
        h.run_steps(5);
        assert_eq!(actions(&h), vec![CompareAction::ToggleField(path_of(CITY))]);
        // The reducer ticked it: the checkbox now shows checked.
        assert_eq!(
            h.get_by_role_and_label(Role::CheckBox, CITY)
                .accesskit_node()
                .toggled(),
            Some(Toggled::True)
        );
        assert_eq!(
            h.get_by_role_and_label(Role::CheckBox, NAME)
                .accesskit_node()
                .toggled(),
            Some(Toggled::False)
        );
        h.get_by_label(&tw("compare.detail.selected_fields", &[("count", "1")]));
    }

    #[test]
    fn a_dollar_named_a_dotted_and_an_unordered_array_field_are_disabled_with_their_reason() {
        let h = harness(with_field_detail());
        assert!(!checkbox_disabled(&h, NAME));
        assert!(!checkbox_disabled(&h, CITY));
        for (path, reason) in [
            (DOLLAR, "compare.apply.field_reason.bad_name"),
            (DOTTED, "compare.apply.field_reason.bad_name"),
            (ANY, "compare.apply.field_reason.any_element"),
        ] {
            assert!(checkbox_disabled(&h, path), "{path}");
            // Shown next to the field without hovering (one line per blocked field).
            let expected = if reason.ends_with("bad_name") { 2 } else { 1 };
            assert_eq!(h.query_all_by_label(&t(reason)).count(), expected, "{path}");
        }
    }

    #[test]
    fn a_blocked_field_checkbox_emits_nothing() {
        let mut h = harness(with_field_detail());
        h.get_by_role_and_label(Role::CheckBox, DOLLAR).click();
        h.run_steps(5);
        assert!(actions(&h).is_empty());
    }

    #[test]
    fn the_gate_disables_every_checkbox_with_the_standard_reason() {
        for (state, reason) in [
            (applied(with_field_detail()), "compare.apply.reason.applied"),
            (applying(with_field_detail()), "compare.apply.reason.busy"),
        ] {
            let mut h = harness(state);
            for path in [NAME, CITY, DOLLAR, DOTTED, ANY] {
                assert!(checkbox_disabled(&h, path), "{reason} {path}");
            }
            h.get_by_role_and_label(Role::CheckBox, NAME).hover();
            h.run_steps(30);
            h.get_by_label(&t(reason));
        }
    }

    // ---- queue buttons ----

    #[test]
    fn with_nothing_ticked_both_buttons_are_disabled_and_say_why() {
        let mut h = harness(with_field_detail());
        for label in [to_target(), to_source()] {
            assert!(button_disabled(&h, &label), "{label}");
        }
        h.get_by_role_and_label(Role::Button, &to_source()).hover();
        h.run_steps(30);
        h.get_by_label(&t("compare.apply.reason.no_fields"));
        h.get_by_label(&tw("compare.detail.selected_fields", &[("count", "0")]));
    }

    #[test]
    fn each_button_emits_its_own_direction() {
        for (label, action) in [(to_target(), TO_T), (to_source(), TO_S)] {
            let mut h = harness(ticked());
            assert!(!button_disabled(&h, &label));
            click(&mut h, &label);
            assert_eq!(actions(&h), vec![CompareAction::AddFieldsToPending(action)]);
        }
    }

    #[test]
    fn ticks_copyable_in_neither_direction_leave_both_buttons_disabled_with_that_reason() {
        let mut state = with_field_detail();
        // The reducer never ticks these; a stale tick must still not enable a button.
        state.compare.field_selection =
            FieldSelection::default().toggled(PAIR, &key(1), &path_of(DOLLAR));
        let mut h = harness(state);
        for label in [to_target(), to_source()] {
            assert!(button_disabled(&h, &label), "{label}");
        }
        h.get_by_role_and_label(Role::Button, &to_target()).hover();
        h.run_steps(30);
        h.get_by_label(&t("compare.apply.reason.not_copyable_here"));
    }

    #[test]
    fn the_gate_disables_the_buttons_with_its_reason() {
        let cases: [(Gate, &str); 4] = [
            (applied, "compare.apply.reason.applied"),
            (applying, "compare.apply.reason.busy"),
            (run_active, "compare.apply.reason.running"),
            (check_out, "compare.apply.reason.checking"),
        ];
        for (make, reason) in cases {
            let mut h = harness(make(ticked()));
            for label in [to_target(), to_source()] {
                assert!(button_disabled(&h, &label), "{reason} {label}");
            }
            h.get_by_role_and_label(Role::Button, &to_target()).hover();
            h.run_steps(30);
            h.get_by_label(&t(reason));
        }
    }

    #[test]
    fn clear_is_disabled_until_a_field_is_ticked_and_then_emits_clear_field_selection() {
        let clear = t("compare.detail.clear_fields");
        assert!(button_disabled(&harness(with_field_detail()), &clear));
        let mut h = harness(ticked());
        assert!(!button_disabled(&h, &clear));
        click(&mut h, &clear);
        assert_eq!(actions(&h), vec![CompareAction::ClearFieldSelection]);
        h.get_by_label(&tw("compare.detail.selected_fields", &[("count", "0")]));
    }

    // ---- pending panel ----

    fn destination() -> String {
        tw(
            "compare.apply.destination",
            &[("connection", "two"), ("namespace", "shop.users")],
        )
    }

    fn fields_line() -> String {
        tw(
            "compare.apply.pending.fields",
            &[
                ("destination", &destination()),
                ("fields", "2"),
                ("documents", "1"),
            ],
        )
    }

    #[test]
    fn the_pending_panel_lists_a_field_copy_with_its_counts_and_removes_it() {
        let mut h = harness(with_fields_pending());
        h.get_by_label(&fields_line());
        h.get_by_label(&tw("compare.apply.pending.total", &[("count", "1")]));
        let named = tw(
            "compare.apply.pending.remove_named",
            &[("line", &fields_line())],
        );
        click(&mut h, &named);
        assert_eq!(actions(&h), vec![CompareAction::RemovePending(0)]);
    }

    // ---- review dialog ----

    fn reviewing(state: AppState) -> AppState {
        super::support::compare_apply_states::reviewing(state)
    }

    #[test]
    fn the_review_shows_the_fields_column_and_the_fields_and_documents_line() {
        let h = harness(reviewing_fields());
        h.get_by_label(&t("compare.apply.kind.fields"));
        h.get_by_label(&tw(
            "compare.apply.review.count_cell",
            &[("kind", &t("compare.apply.kind.fields")), ("count", "1")],
        ));
        h.get_by_label(&tw(
            "compare.apply.review.fields_line",
            &[("fields", "2"), ("documents", "1")],
        ));
        h.get_by_label(&tw("compare.apply.review.total", &[("count", "1")]));
        h.get_by_label(&format!(
            "⚠ {}",
            t("compare.apply.warning.field_copy_keeps_rest")
        ));
    }

    #[test]
    fn a_field_only_review_needs_no_typing_and_apply_confirms_once() {
        let mut h = harness(reviewing_fields());
        assert!(h.query_by_role(Role::TextInput).is_none());
        let apply = t("compare.apply.review.apply");
        assert!(!button_disabled(&h, &apply));
        click(&mut h, &apply);
        let confirms = actions(&h)
            .iter()
            .filter(|a| **a == CompareAction::ConfirmApply)
            .count();
        assert_eq!(confirms, 1);
    }

    #[test]
    fn enter_and_space_right_after_opening_a_field_review_confirm_nothing() {
        let mut h = harness(reviewing_fields());
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
        h.key_press(egui::Key::Space);
        h.run_steps(3);
        assert!(!actions(&h).contains(&CompareAction::ConfirmApply));
    }

    #[test]
    fn a_review_that_also_deletes_still_requires_the_typed_text() {
        let h = harness(reviewing(with_fields_and_delete_pending()));
        h.get_by_role(Role::TextInput);
        assert!(button_disabled(&h, &t("compare.apply.review.apply")));
        h.get_by_label(&tw(
            "compare.apply.review.fields_line",
            &[("fields", "2"), ("documents", "1")],
        ));
    }

    // ---- results table ----

    #[test]
    fn the_row_of_an_open_nan_keyed_document_is_highlighted() {
        let nan = KeyTuple(vec![Bson::Double(f64::NAN)]);
        let state = act(
            finished(),
            CompareAction::SelectCategory(Category::OnlyInTarget),
        );
        let page = CompareEvent::Rows {
            session: 1,
            pair: PAIR,
            category: Category::OnlyInTarget,
            offset: 0,
            total: 1,
            rows: vec![RowView {
                key: nan.clone(),
                has_source: false,
                has_target: true,
            }],
        };
        let state = step(state, Input::Event(Event::Compare(page)));
        let state = act(state, CompareAction::Select(nan.clone()));
        let detail = CompareEvent::Detail {
            session: 1,
            pair: PAIR,
            key: nan.clone(),
            source: Some(bson::doc! {"name": "x"}),
            target: Some(bson::doc! {"name": "y"}),
            changes: Vec::new(),
        };
        let state = step(state, Input::Event(Event::Compare(detail)));
        assert!(state.compare.detail.is_some(), "the NaN detail is open");
        let h = harness(state);
        let label = key_label(&nan);
        let row = h
            .get_all_by_label(&label)
            .find(|n| n.accesskit_node().role() != Role::CheckBox)
            .expect("the row");
        assert_eq!(row.accesskit_node().toggled(), Some(Toggled::True));
    }

    // ---- tab, focus order, several lines ----

    #[test]
    fn a_document_that_now_exists_on_both_sides_still_cannot_be_copied_from_another_tab() {
        let state = act(
            finished(),
            CompareAction::SelectCategory(Category::OnlyInTarget),
        );
        let state = rows_of(state, Category::OnlyInTarget, 2);
        let state = act(state, CompareAction::Select(key(1)));
        let detail = CompareEvent::Detail {
            session: 1,
            pair: PAIR,
            key: key(1),
            source: Some(bson::doc! {"name": "x"}),
            target: Some(bson::doc! {"name": "y"}),
            changes: super::support::compare_field_states::changes(),
        };
        let state = step(state, Input::Event(Event::Compare(detail)));
        let mut h = harness(state);
        for path in [NAME, CITY, DOLLAR] {
            assert!(checkbox_disabled(&h, path), "{path}");
        }
        for label in [to_target(), to_source()] {
            assert!(button_disabled(&h, &label), "{label}");
        }
        h.get_by_role_and_label(Role::CheckBox, NAME).hover();
        h.run_steps(30);
        h.get_by_label(&t("compare.apply.reason.not_different_tab"));
    }

    /// The label of the control that has keyboard focus, among the field controls.
    fn focused(h: &Harness<'_, Run>) -> Option<String> {
        let buttons = [to_target(), to_source(), t("compare.detail.clear_fields")];
        let on = |role, label: &str| {
            h.query_by_role_and_label(role, label)
                .is_some_and(|n| n.accesskit_node().is_focused())
        };
        buttons
            .iter()
            .find(|l| on(Role::Button, l))
            .cloned()
            .or_else(|| {
                [NAME, CITY, DOLLAR, DOTTED, ANY]
                    .iter()
                    .find(|l| on(Role::CheckBox, l))
                    .map(|l| l.to_string())
            })
    }

    #[test]
    fn tab_visits_the_buttons_then_the_checkboxes_row_by_row() {
        let mut h = harness(ticked());
        let mut order = Vec::new();
        for _ in 0..40 {
            h.key_press(egui::Key::Tab);
            h.run_steps(2);
            let label = focused(&h);
            if label.is_some() && order.last() != label.as_ref() {
                order.extend(label);
            }
        }
        let expected = [
            to_target(),
            to_source(),
            t("compare.detail.clear_fields"),
            NAME.to_string(),
            CITY.to_string(),
        ];
        assert_eq!(&order[..5], &expected, "{order:?}");
    }

    #[test]
    fn removing_the_second_of_two_field_lines_emits_index_one() {
        // A second document (key 2), queued the other way: one line per document.
        let state = act(with_fields_pending(), CompareAction::Select(key(2)));
        let detail = CompareEvent::Detail {
            session: 1,
            pair: PAIR,
            key: key(2),
            source: Some(bson::doc! {"name": "x"}),
            target: Some(bson::doc! {"name": "y"}),
            changes: super::support::compare_field_states::changes(),
        };
        let state = step(state, Input::Event(Event::Compare(detail)));
        let state = super::support::compare_field_states::tick(state, NAME);
        let state = act(state, CompareAction::AddFieldsToPending(TO_S));
        assert_eq!(state.compare.pending.items().len(), 2, "two lines");
        let line = tw(
            "compare.apply.pending.fields",
            &[
                (
                    "destination",
                    &tw(
                        "compare.apply.destination",
                        &[("connection", "one"), ("namespace", "shop.users")],
                    ),
                ),
                ("fields", "1"),
                ("documents", "1"),
            ],
        );
        let mut h = harness(state);
        let named = tw("compare.apply.pending.remove_named", &[("line", &line)]);
        click(&mut h, &named);
        assert_eq!(actions(&h), vec![CompareAction::RemovePending(1)]);
    }
}
