//! The pending panel, the review dialog, the progress panel and the result panel: what they
//! show and which action each control emits. Fast, offline, no GPU.
mod support;

mod compare_apply_dialog {
    use super::support::compare_apply_states::{
        ONLY_IN_TARGET, PAIR, act, applied, check_out, reviewing, run_active, with_apply,
        with_delete, with_deletes_on_both_sides, with_insert, with_replace,
    };
    use curlybrace_core::compare::apply::{ApplyReport, IdPolicy, OpFailure};
    use curlybrace_core::compare::key::KeyTuple;
    use curlybrace_core::error::CoreError;
    use curlybrace_ui::I18n;
    use curlybrace_ui::app::compare::CompareAction;
    use curlybrace_ui::app::state::AppState;
    use curlybrace_ui::viewmodel::compare::{ApplyRow, ApplyUi, RequiredText, RowAction};
    use curlybrace_ui::views::compare::show_compare;
    use eframe::egui;
    use eframe::egui::accesskit::Role;
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    /// The app (actions are fed back through the reducer, as the real app does) and every action
    /// the views emitted.
    struct Run {
        app: AppState,
        actions: Vec<CompareAction>,
    }

    fn harness(state: AppState) -> Harness<'static, Run> {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1100.0, 1600.0))
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

    fn click(harness: &mut Harness<'_, Run>, label: &str) {
        harness.get_by_role_and_label(Role::Button, label).click();
        harness.run_steps(5);
    }

    fn disabled(harness: &Harness<'_, Run>, label: &str) -> bool {
        harness
            .get_by_role_and_label(Role::Button, label)
            .accesskit_node()
            .is_disabled()
    }

    fn type_confirmation(harness: &mut Harness<'_, Run>, text: &str) {
        let input = harness.get_by_role(Role::TextInput);
        input.focus();
        harness.run_steps(5);
        harness.get_by_role(Role::TextInput).type_text(text);
        harness.run_steps(5);
    }

    /// Empties the typed confirmation (as if the person selected and deleted it).
    fn clear_text(harness: &mut Harness<'_, Run>) {
        let run = harness.state_mut();
        let app = std::mem::replace(&mut run.app, AppState::new(Vec::new()));
        run.app = act(app, CompareAction::SetConfirmText(String::new()));
        harness.run_steps(5);
    }

    fn destination(connection: &str, namespace: &str) -> String {
        tw(
            "compare.apply.destination",
            &[("connection", connection), ("namespace", namespace)],
        )
    }

    // ---- pending panel ----

    fn replace_line() -> String {
        tw(
            "compare.apply.pending.copy",
            &[
                ("destination", &destination("two", "shop.users")),
                ("count", "2"),
            ],
        )
    }

    #[test]
    fn the_pending_panel_names_the_destination_and_the_count() {
        let harness = harness(with_replace());
        let line = tw(
            "compare.apply.pending.copy",
            &[
                ("destination", &destination("two", "shop.users")),
                ("count", "2"),
            ],
        );
        harness.get_by_label(&line);
        harness.get_by_label(&tw("compare.apply.pending.total", &[("count", "2")]));
    }

    #[test]
    fn a_delete_line_says_delete_from_its_destination() {
        let harness = harness(with_delete());
        let line = tw(
            "compare.apply.pending.delete",
            &[
                ("destination", &destination("two", "shop.users")),
                ("count", &ONLY_IN_TARGET.min(2).to_string()),
            ],
        );
        harness.get_by_label(&line);
    }

    #[test]
    fn remove_clear_and_review_emit_their_actions() {
        let mut h = harness(with_replace());
        let named = tw(
            "compare.apply.pending.remove_named",
            &[("line", &replace_line())],
        );
        click(&mut h, &named);
        assert_eq!(actions(&h), vec![CompareAction::RemovePending(0)]);

        let mut h = harness(with_replace());
        click(&mut h, &t("compare.apply.pending.clear"));
        assert_eq!(actions(&h), vec![CompareAction::ClearPending]);

        let mut h = harness(with_replace());
        click(&mut h, &t("compare.apply.pending.review"));
        assert_eq!(
            actions(&h),
            vec![CompareAction::ReviewApply {
                word: t("compare.apply.confirm_word")
            }]
        );
    }

    #[test]
    fn review_is_disabled_with_its_reason_when_it_cannot_fire() {
        let review = t("compare.apply.pending.review");
        for (state, reason) in [
            (run_active(with_replace()), "compare.apply.reason.running"),
            (check_out(with_replace()), "compare.apply.reason.checking"),
            (applied(with_replace()), "compare.apply.reason.applied"),
        ] {
            let mut h = harness(state);
            assert!(disabled(&h, &review), "{reason}");
            h.get_by_label(&t(reason));
            h.get_by_role_and_label(Role::Button, &review).click();
            h.run_steps(5);
            assert!(actions(&h).is_empty(), "a disabled Review emits nothing");
        }
    }

    #[test]
    fn nothing_pending_shows_no_panel() {
        let h = harness(super::support::compare_apply_states::finished());
        assert!(
            h.query_by_role_and_label(Role::Button, &t("compare.apply.pending.review"))
                .is_none()
        );
    }

    // ---- review dialog ----

    #[test]
    fn the_dialog_shows_destinations_and_counts() {
        let h = harness(reviewing(with_insert()));
        h.get_by_label(&t("compare.apply.review.title"));
        h.get_by_label(&tw(
            "compare.apply.review.destination_line",
            &[
                ("action", &t("compare.apply.action.copy_to_target")),
                ("destination", &destination("two", "shop.users")),
            ],
        ));
        h.get_by_label(&tw("compare.apply.review.total", &[("count", "2")]));
    }

    #[test]
    fn apply_is_disabled_until_the_collection_name_is_typed_exactly() {
        let apply = t("compare.apply.review.apply");
        let mut h = harness(reviewing(with_delete()));
        h.get_by_label(&t("compare.apply.review.confirm_prompt"));
        h.get_by_label("users");
        assert!(disabled(&h, &apply));
        for wrong in ["user", "Users", "users2"] {
            type_confirmation(&mut h, wrong);
            assert!(disabled(&h, &apply), "{wrong}");
            // clear for the next attempt
            clear_text(&mut h);
        }
        type_confirmation(&mut h, "users");
        assert!(!disabled(&h, &apply));
    }

    #[test]
    fn typing_emits_set_confirm_text() {
        let mut h = harness(reviewing(with_delete()));
        type_confirmation(&mut h, "u");
        assert_eq!(
            actions(&h).last(),
            Some(&CompareAction::SetConfirmText("u".into()))
        );
    }

    #[test]
    fn the_database_variant_asks_for_the_database_name() {
        let state = reviewing(with_delete());
        let mut review = state.compare.apply.review().unwrap().clone();
        review.required = Some(RequiredText::Literal("shop".into()));
        let mut h = harness(with_apply(state, ApplyUi::Reviewing(review)));
        h.get_by_label(&t("compare.apply.review.confirm_prompt"));
        h.get_by_label("shop");
        let apply = t("compare.apply.review.apply");
        type_confirmation(&mut h, "users");
        assert!(disabled(&h, &apply));
        clear_text(&mut h);
        type_confirmation(&mut h, "shop");
        assert!(!disabled(&h, &apply));
    }

    #[test]
    fn the_word_variant_shows_the_localized_word_and_is_case_sensitive() {
        let state = reviewing(with_deletes_on_both_sides());
        assert_eq!(
            state.compare.apply.review().unwrap().required,
            Some(RequiredText::Word)
        );
        let mut h = harness(state);
        h.get_by_label(&t("compare.apply.review.confirm_prompt"));
        h.get_by_label("delete");
        let apply = t("compare.apply.review.apply");
        type_confirmation(&mut h, "Delete");
        assert!(disabled(&h, &apply));
        clear_text(&mut h);
        type_confirmation(&mut h, "delete");
        assert!(!disabled(&h, &apply));
    }

    #[test]
    fn apply_emits_confirm_once() {
        let mut h = harness(reviewing(with_delete()));
        type_confirmation(&mut h, "users");
        click(&mut h, &t("compare.apply.review.apply"));
        h.run_steps(5);
        let confirms = actions(&h)
            .iter()
            .filter(|a| **a == CompareAction::ConfirmApply)
            .count();
        assert_eq!(confirms, 1);
    }

    #[test]
    fn a_replace_needs_no_typing_and_apply_is_enabled() {
        let h = harness(reviewing(with_replace()));
        assert!(h.query_by_role(Role::TextInput).is_none());
        assert!(!disabled(&h, &t("compare.apply.review.apply")));
    }

    #[test]
    fn cancel_and_escape_emit_cancel_review() {
        let mut h = harness(reviewing(with_replace()));
        click(&mut h, &t("compare.apply.review.cancel"));
        assert_eq!(actions(&h), vec![CompareAction::CancelReview]);

        let mut h = harness(reviewing(with_replace()));
        h.key_press(egui::Key::Escape);
        h.run_steps(5);
        assert_eq!(actions(&h), vec![CompareAction::CancelReview]);
    }

    #[test]
    fn enter_in_the_text_field_emits_nothing_but_the_typing() {
        let mut h = harness(reviewing(with_delete()));
        type_confirmation(&mut h, "users");
        let before = actions(&h).len();
        h.key_press(egui::Key::Enter);
        h.run_steps(5);
        assert_eq!(actions(&h).len(), before, "Enter confirms nothing");
        assert!(!actions(&h).contains(&CompareAction::ConfirmApply));
    }

    #[test]
    fn the_id_policy_toggle_exists_only_with_inserts() {
        let keep = t("compare.apply.id_policy.keep");
        let generate = t("compare.apply.id_policy.generate");
        let with = harness(reviewing(with_insert()));
        with.get_by_label(&keep);
        let mut with = with;
        with.get_by_label(&generate).click();
        with.run_steps(5);
        assert_eq!(
            actions(&with),
            vec![CompareAction::SetIdPolicy(IdPolicy::Generate)]
        );
        let without = harness(reviewing(with_replace()));
        assert!(without.query_by_label(&keep).is_none());
        assert!(without.query_by_label(&generate).is_none());
    }

    #[test]
    fn warnings_are_rendered() {
        let state = act(
            reviewing(with_insert()),
            CompareAction::SetIdPolicy(IdPolicy::Generate),
        );
        let h = harness(state);
        h.get_by_label_contains(&t("compare.apply.warning.generated_ids"));
    }

    // ---- running and finished ----

    fn running(done: u64, cancel_requested: bool) -> AppState {
        let rows = vec![ApplyRow {
            pair: PAIR,
            entry: 0,
            done,
            total: 4,
            report: None,
        }];
        let apply = ApplyUi::Running {
            run: 9,
            total: 4,
            rows,
            cancel_requested,
        };
        with_apply(with_replace(), apply)
    }

    #[test]
    fn the_running_panel_shows_progress_and_cancel_emits_cancel_apply() {
        let mut h = harness(running(3, false));
        h.get_by_label(&t("compare.apply.running.title"));
        h.get_by_label(&tw(
            "compare.apply.running.progress",
            &[("done", "3"), ("total", "4")],
        ));
        click(&mut h, &t("compare.apply.running.cancel"));
        assert_eq!(actions(&h), vec![CompareAction::CancelApply]);
    }

    #[test]
    fn once_cancel_is_requested_the_button_is_disabled_and_says_cancelling() {
        let h = harness(running(1, true));
        assert!(disabled(&h, &t("compare.apply.running.cancel")));
        h.get_by_label(&t("compare.apply.running.cancelling"));
    }

    fn report(
        applied: u64,
        stale: u64,
        failed: u64,
        failures: Vec<(i32, CoreError)>,
    ) -> ApplyReport {
        ApplyReport {
            applied,
            stale,
            failed,
            failures: failures
                .into_iter()
                .map(|(n, error)| OpFailure {
                    key: KeyTuple(vec![bson::Bson::Int32(n)]),
                    error,
                })
                .collect(),
            ..ApplyReport::default()
        }
    }

    fn finished(report: ApplyReport, cancelled: bool, requested: u64) -> AppState {
        let apply = ApplyUi::Finished {
            reports: vec![(PAIR, report)],
            cancelled,
            requested,
        };
        with_apply(with_replace(), apply)
    }

    fn outcome(key: &str, count: u64) -> String {
        tw(key, &[("count", &count.to_string())])
    }

    #[test]
    fn the_finished_panel_counts_add_up_with_labels_and_dismiss_emits() {
        let state = finished(report(4, 2, 1, vec![(7, CoreError::NotFound)]), false, 10);
        let mut h = harness(state);
        // 4 + 2 + 1 are reported; the 3 writes no report accounts for count as not run.
        for (icon, key, n) in [
            ("✔", "compare.apply.outcome.applied", 4),
            ("⚠", "compare.apply.outcome.stale", 2),
            ("—", "compare.apply.outcome.not_applicable", 0),
            ("✖", "compare.apply.outcome.failed", 1),
            ("⏹", "compare.apply.outcome.not_run", 3),
        ] {
            h.get_by_label(&format!("{icon} {}", outcome(key, n)));
        }
        h.get_by_label(&tw("compare.apply.finished.requested", &[("count", "10")]));
        h.get_by_label(&t("compare.apply.finished.rerun_note"));
        click(&mut h, &t("compare.apply.finished.dismiss"));
        assert_eq!(actions(&h), vec![CompareAction::DismissApplyResult]);
    }

    #[test]
    fn failures_show_the_key_and_the_typed_error_text() {
        let failures = vec![
            (7, CoreError::NotFound),
            (8, CoreError::ApplyOutcomeUnknown),
        ];
        let h = harness(finished(report(0, 0, 2, failures), false, 2));
        h.get_by_label("7");
        h.get_by_label(&I18n::english().error(&CoreError::NotFound));
        h.get_by_label(&I18n::english().error(&CoreError::ApplyOutcomeUnknown));
        h.get_by_label(&t("compare.apply.finished.outcome_unknown_note"));
    }

    #[test]
    fn without_an_unknown_outcome_there_is_no_unknown_note() {
        let h = harness(finished(
            report(0, 0, 1, vec![(7, CoreError::NotFound)]),
            false,
            1,
        ));
        assert!(
            h.query_by_label(&t("compare.apply.finished.outcome_unknown_note"))
                .is_none()
        );
    }

    #[test]
    fn failures_are_capped_across_pairs_and_the_more_note_shows_because_the_list_was_cut() {
        let first: Vec<_> = (1001..=1080).map(|n| (n, CoreError::NotFound)).collect();
        let second: Vec<_> = (1081..=1160).map(|n| (n, CoreError::NotFound)).collect();
        let apply = ApplyUi::Finished {
            reports: vec![
                (PAIR, report(0, 0, 80, first)),
                (PAIR + 40, report(0, 0, 80, second)),
            ],
            cancelled: false,
            requested: 160,
        };
        let h = harness(with_apply(with_replace(), apply));
        h.get_by_label("1100");
        assert!(h.query_by_label("1101").is_none());
        h.get_by_label(&t("compare.apply.finished.more_failures"));
    }

    #[test]
    fn the_more_note_also_shows_when_the_core_cut_the_list() {
        let mut cut = report(0, 0, 150, vec![(7, CoreError::NotFound)]);
        cut.more_failures = true;
        let h = harness(finished(cut, false, 150));
        h.get_by_label(&t("compare.apply.finished.more_failures"));
        h.get_by_label(&t("compare.apply.finished.maybe_unknown_note"));
    }

    #[test]
    fn no_more_note_when_nothing_was_cut() {
        let h = harness(finished(
            report(0, 0, 1, vec![(7, CoreError::NotFound)]),
            false,
            1,
        ));
        assert!(
            h.query_by_label(&t("compare.apply.finished.more_failures"))
                .is_none()
        );
        assert!(
            h.query_by_label(&t("compare.apply.finished.maybe_unknown_note"))
                .is_none()
        );
    }

    #[test]
    fn a_cancelled_apply_is_explained() {
        let note = t("compare.apply.finished.cancelled_note");
        let cancelled = harness(finished(report(1, 0, 0, vec![]), true, 4));
        cancelled.get_by_label(&note);
        let done = harness(finished(report(4, 0, 0, vec![]), false, 4));
        assert!(done.query_by_label(&note).is_none());
    }

    // ---- review fixes ----

    #[test]
    fn labels_come_from_the_run_not_from_the_entry_edited_since() {
        let edit = |mut state: AppState| {
            state.compare.pairs.entries[0].target.collection = "renamed".into();
            state
        };
        let run = pair_label("shop.users", "shop.users");
        let edited = pair_label("shop.users", "shop.renamed");
        let panel = harness(edit(with_replace()));
        panel.get_by_label(&run);
        // The pair table legitimately shows the edited entry; the dialog adds no such label.
        let before = panel.get_all_by_label(&edited).count();
        let dialog = harness(edit(reviewing(with_replace())));
        assert_eq!(dialog.get_all_by_label(&edited).count(), before);
        assert!(dialog.get_all_by_label(&run).count() > panel.get_all_by_label(&run).count());
    }

    fn pair_label(source: &str, target: &str) -> String {
        tw(
            "compare.pairs.pair_label",
            &[("source", source), ("target", target)],
        )
    }

    #[test]
    fn long_destinations_wrap_inside_the_dialog() {
        let mut state = with_delete();
        let long = "x".repeat(70);
        state.conn.connections[1].name = format!("{long} {long}");
        let h = harness(reviewing(state));
        let title = h.get_by_label(&t("compare.apply.review.title")).rect();
        let line = h
            .get_all_by_label_contains(&long)
            .map(|n| n.rect())
            .filter(|r| r.top() > title.top())
            .collect::<Vec<_>>();
        assert!(!line.is_empty());
        for rect in line {
            assert!(
                rect.right() <= title.left() + 620.0,
                "{rect:?} vs {title:?}"
            );
        }
    }

    #[test]
    fn missing_connection_names_use_a_translated_fallback() {
        let mut state = with_replace();
        state.conn.connections.clear();
        let h = harness(state);
        let destination = destination(&t("compare.apply.unknown_connection"), "shop.users");
        let line = tw(
            "compare.apply.pending.copy",
            &[("destination", &destination), ("count", "2")],
        );
        h.get_by_label(&line);
    }

    #[test]
    fn a_whole_tab_item_says_every_document_of_the_tab() {
        let state = act(with_replace(), CompareAction::ClearPending);
        let state = act(state, CompareAction::SelectAllInCategory);
        let state = act(state, CompareAction::AddToPending(RowAction::CopyToTarget));
        let count = super::support::compare_apply_states::DIFFERENT.to_string();
        let line = tw(
            "compare.apply.pending.copy_all",
            &[
                ("destination", &destination("two", "shop.users")),
                ("count", &count),
            ],
        );
        harness(state).get_by_label(&line);
        let line = tw(
            "compare.apply.pending.delete_all",
            &[("destination", "x"), ("count", "1")],
        );
        assert!(line.contains("every document"));
    }

    #[test]
    fn the_pending_panel_behind_the_dialog_does_not_claim_an_apply_is_running() {
        let h = harness(reviewing(with_replace()));
        assert!(h.query_by_label(&t("compare.apply.reason.busy")).is_none());
        let h = harness(running(1, false));
        h.get_by_label(&t("compare.apply.reason.busy"));
    }

    #[test]
    fn the_dialog_names_its_count_cells_and_puts_them_in_their_columns() {
        let h = harness(reviewing(with_insert()));
        for (kind, count) in [("insert", "2"), ("replace", "0"), ("delete", "0")] {
            let name = t(&format!("compare.apply.kind.{kind}"));
            let cell = tw(
                "compare.apply.review.count_cell",
                &[("kind", &name), ("count", count)],
            );
            let header = h.get_by_label(&name).rect();
            let cell = h.get_by_label(&cell).rect();
            assert!((header.left() - cell.left()).abs() < 1.0, "{kind}");
        }
    }

    #[test]
    fn the_keep_radio_is_the_default_choice() {
        let h = harness(reviewing(with_insert()));
        let keep_label = t("compare.apply.id_policy.keep");
        let generate_label = t("compare.apply.id_policy.generate");
        let keep = h.get_by_label(&keep_label);
        assert_eq!(
            keep.accesskit_node().toggled(),
            Some(egui::accesskit::Toggled::True)
        );
        let generate = h.get_by_label(&generate_label);
        assert_eq!(
            generate.accesskit_node().toggled(),
            Some(egui::accesskit::Toggled::False)
        );
    }

    #[test]
    fn surrounding_spaces_in_the_typed_text_are_ignored() {
        let mut h = harness(reviewing(with_delete()));
        type_confirmation(&mut h, " users ");
        assert!(!disabled(&h, &t("compare.apply.review.apply")));
    }

    #[test]
    fn enter_and_space_right_after_opening_confirm_nothing() {
        let mut h = harness(reviewing(with_replace()));
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
        h.key_press(egui::Key::Space);
        h.run_steps(3);
        assert!(!actions(&h).contains(&CompareAction::ConfirmApply));
    }

    #[test]
    fn review_resolvable_agrees_with_can_review() {
        let states: Vec<AppState> = vec![
            with_replace(),
            with_insert(),
            with_delete(),
            with_deletes_on_both_sides(),
            reviewing(with_replace()),
            run_active(with_replace()),
            check_out(with_replace()),
            applied(with_replace()),
            running(1, false),
            super::support::compare_apply_states::finished(),
            act(with_replace(), CompareAction::ClearPending),
        ];
        for (i, state) in states.iter().enumerate() {
            assert_eq!(
                state.compare.review_resolvable(),
                state.compare.can_review(),
                "state {i}"
            );
        }
    }
}
