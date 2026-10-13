//! The export controls: the two entry buttons and their reasons, the options modal, the waiting,
//! running and result states. What each control shows and which action it emits. Fast, offline,
//! no GPU, and no native dialog: the views only emit `ChooseExportFile`.
mod support;

mod compare_export {
    use super::support::compare_apply_states::{act, applying, check_out, run_active};
    use super::support::compare_export_states::{
        choosing, copied, finished_with, out_path, overview, overview_without_results, plan,
        plan_deletes, plan_mixed, plan_modal, plan_with_inserts, progressed, report_modal,
        report_modal_with_rows, running_clipboard, running_plan, running_report,
    };
    use super::support::compare_multi_states::pair_results;
    use curlybrace_core::compare::apply::IdPolicy;
    use curlybrace_core::compare::protocol::{ExportFormat, ExportOutcome};
    use curlybrace_core::connection::ConnectionConfig;
    use curlybrace_ui::I18n;
    use curlybrace_ui::app::compare::CompareAction;
    use curlybrace_ui::app::state::{AppState, Workspace};
    use curlybrace_ui::viewmodel::compare::{ExportBlock, ExportKind};
    use curlybrace_ui::views::compare::show_compare;
    use eframe::egui;
    use eframe::egui::accesskit::Role;
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

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
                    app: AppState {
                        workspace: Workspace::Compare,
                        ..state
                    },
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

    fn actions(h: &Harness<'_, Run>) -> Vec<CompareAction> {
        h.state().actions.clone()
    }

    fn click(h: &mut Harness<'_, Run>, label: &str) {
        h.get_by_role_and_label(Role::Button, label).click();
        h.run_steps(5);
    }

    fn click_role(h: &mut Harness<'_, Run>, role: Role, label: &str) {
        h.get_by_role_and_label(role, label).click();
        h.run_steps(5);
    }

    fn disabled(h: &Harness<'_, Run>, label: &str) -> bool {
        h.get_by_role_and_label(Role::Button, label)
            .accesskit_node()
            .is_disabled()
    }

    fn shown(h: &Harness<'_, Run>, label: &str) -> bool {
        h.query_all_by_label(label).count() > 0
    }

    fn has_button(h: &Harness<'_, Run>, key: &str) -> bool {
        h.query_by_role_and_label(Role::Button, &t(key)).is_some()
    }

    #[allow(deprecated)]
    fn press(h: &mut Harness<'_, Run>, key: egui::Key) {
        let node = h
            .query_all_by_role(Role::Label)
            .next()
            .expect("something is drawn");
        node.key_down(key);
        node.key_up(key);
        h.run_steps(5);
    }

    fn reason(block: ExportBlock) -> String {
        t(block.i18n_key())
    }

    // ---- the entry buttons ----

    #[test]
    fn export_changes_opens_the_plan_modal() {
        let mut h = harness(plan());
        click(&mut h, &t("compare.export.open_plan"));
        assert_eq!(
            actions(&h),
            vec![CompareAction::OpenExport(ExportKind::Plan)]
        );
        h.get_by_label(&t("compare.export.title.plan"));
    }

    #[test]
    fn export_changes_is_disabled_with_its_reason_when_it_cannot_fire() {
        let button = t("compare.export.open_plan");
        for (state, block) in [
            (run_active(plan()), ExportBlock::RunActive),
            (check_out(plan()), ExportBlock::CheckInFlight),
            (applying(plan()), ExportBlock::ApplyBusy),
            (running_plan(), ExportBlock::ExportBusy),
        ] {
            let mut h = harness(state);
            assert!(disabled(&h, &button), "{block:?}");
            assert!(shown(&h, &reason(block)), "{block:?} shows its reason");
            let before = actions(&h).len();
            h.get_by_role_and_label(Role::Button, &button).click();
            h.run_steps(5);
            assert_eq!(actions(&h).len(), before, "a disabled button emits nothing");
        }
    }

    #[test]
    fn export_report_opens_the_report_modal() {
        let mut h = harness(overview());
        click(&mut h, &t("compare.export.open_report"));
        assert_eq!(
            actions(&h),
            vec![CompareAction::OpenExport(ExportKind::Report)]
        );
        h.get_by_label(&t("compare.export.title.report"));
    }

    #[test]
    fn export_report_is_disabled_with_its_reason_when_it_cannot_fire() {
        let button = t("compare.export.open_report");
        for (state, block) in [
            (run_active(overview()), ExportBlock::RunActive),
            (check_out(overview()), ExportBlock::CheckInFlight),
            (overview_without_results(), ExportBlock::NoCompletedPair),
            (running_report(), ExportBlock::ExportBusy),
        ] {
            let mut h = harness(state);
            assert!(disabled(&h, &button), "{block:?}");
            assert!(shown(&h, &reason(block)), "{block:?} shows its reason");
            let before = actions(&h).len();
            h.get_by_role_and_label(Role::Button, &button).click();
            h.run_steps(5);
            assert_eq!(actions(&h).len(), before, "a disabled button emits nothing");
        }
    }

    // ---- the plan modal ----

    #[test]
    fn the_plan_modal_shows_counts_and_warnings() {
        let h = harness(plan_modal());
        h.get_by_label(&t("compare.export.title.plan"));
        assert!(shown(
            &h,
            &tw("compare.apply.pending.total", &[("count", "2")])
        ));
        for key in [
            "compare.export.warning.data",
            "compare.export.warning.stale",
            "compare.export.warning.skipped",
        ] {
            assert!(shown(&h, &t(key)), "{key}");
        }
    }

    #[test]
    fn the_format_radios_emit_the_chosen_format() {
        let mut h = harness(plan_modal());
        click_role(&mut h, Role::RadioButton, &t("compare.export.format.json"));
        assert_eq!(
            actions(&h),
            vec![CompareAction::SetExportFormat(ExportFormat::JsonPlan)]
        );
        click_role(
            &mut h,
            Role::RadioButton,
            &t("compare.export.format.mongosh"),
        );
        assert_eq!(
            actions(&h).last(),
            Some(&CompareAction::SetExportFormat(ExportFormat::Mongosh))
        );
    }

    #[test]
    fn the_script_option_shows_the_mixed_destination_reason() {
        let state = super::support::compare_export_states::open(plan_mixed(), ExportKind::Plan);
        let h = harness(state);
        assert!(shown(&h, &reason(ExportBlock::MixedSides)));
        let (mongosh, json) = (
            t("compare.export.format.mongosh"),
            t("compare.export.format.json"),
        );
        let script = h.get_by_role_and_label(Role::RadioButton, &mongosh);
        assert!(script.accesskit_node().is_disabled());
        let json = h.get_by_role_and_label(Role::RadioButton, &json);
        assert!(!json.accesskit_node().is_disabled());
        assert!(!disabled(&h, &t("compare.export.choose_file")));
    }

    #[test]
    fn the_script_option_has_no_reason_when_writes_go_one_way() {
        let state = super::support::compare_export_states::open(plan_deletes(), ExportKind::Plan);
        let h = harness(state);
        assert!(!shown(&h, &reason(ExportBlock::MixedSides)));
    }

    #[test]
    fn the_id_policy_shows_only_when_documents_are_inserted() {
        let h = harness(plan_modal());
        assert!(!shown(&h, &t("compare.apply.id_policy.label")));

        let state =
            super::support::compare_export_states::open(plan_with_inserts(), ExportKind::Plan);
        let mut h = harness(state);
        h.get_by_label(&t("compare.apply.id_policy.label"));
        click_role(
            &mut h,
            Role::RadioButton,
            &t("compare.apply.id_policy.generate"),
        );
        assert_eq!(
            actions(&h),
            vec![CompareAction::SetExportIdPolicy(IdPolicy::Generate)]
        );
    }

    #[test]
    fn typing_a_label_emits_it_and_enter_emits_nothing() {
        let mut h = harness(plan_modal());
        let input = h.get_by_role(Role::TextInput);
        input.focus();
        h.run_steps(5);
        h.get_by_role(Role::TextInput).type_text("ticket 7");
        h.run_steps(5);
        assert_eq!(
            actions(&h).last(),
            Some(&CompareAction::SetExportLabel("ticket 7".into()))
        );
        let before = actions(&h).len();
        press(&mut h, egui::Key::Enter);
        assert_eq!(actions(&h).len(), before, "Enter in the label does nothing");
    }

    #[test]
    fn the_label_box_is_named_for_screen_readers() {
        let h = harness(plan_modal());
        let caption = t("compare.export.label.caption");
        h.get_by_label(&caption);
        let input = h.get_by_role(Role::TextInput);
        assert!(input.accesskit_node().labelled_by().next().is_some());
    }

    #[test]
    fn a_hostile_label_renders_as_text() {
        let hostile = "a\"b\n$(rm -rf /)\u{2028}</script>'; DROP\r\n\0";
        let state = act(plan_modal(), CompareAction::SetExportLabel(hostile.into()));
        let h = harness(state);
        h.get_by_label(&t("compare.export.title.plan"));
        let state = act(
            report_modal(),
            CompareAction::SetExportLabel(hostile.into()),
        );
        let h = harness(state);
        h.get_by_label(&t("compare.export.title.report"));
    }

    #[test]
    fn choose_file_emits_one_action_and_cancel_closes() {
        let mut h = harness(plan_modal());
        click(&mut h, &t("compare.export.choose_file"));
        assert_eq!(actions(&h), vec![CompareAction::ChooseExportFile]);

        let mut h = harness(plan_modal());
        click(&mut h, &t("compare.export.cancel"));
        assert_eq!(actions(&h), vec![CompareAction::CloseExport]);
    }

    #[test]
    fn escape_closes_the_modal_only_while_configuring() {
        let mut h = harness(plan_modal());
        press(&mut h, egui::Key::Escape);
        assert_eq!(actions(&h), vec![CompareAction::CloseExport]);

        for state in [choosing(plan_modal()), running_plan()] {
            let mut h = harness(state);
            press(&mut h, egui::Key::Escape);
            assert!(
                actions(&h).is_empty(),
                "Escape must not touch a busy export"
            );
        }
    }

    // ---- the report modal ----

    /// Three finished pairs on the overview.
    fn three_pairs() -> AppState {
        let mut state = AppState::new(vec![
            ConnectionConfig::new("one", "mongodb://a"),
            ConnectionConfig::new("two", "mongodb://b"),
        ]);
        pair_results(&mut state);
        act(state, CompareAction::BackToOverview)
    }

    #[test]
    fn the_report_modal_lists_the_pairs_options_and_warnings() {
        let outside = harness(three_pairs())
            .query_all_by_role(Role::CheckBox)
            .count();
        let state = act(three_pairs(), CompareAction::OpenExport(ExportKind::Report));
        assert!(state.compare.export.configuring().is_some());
        let mut h = harness(state);
        h.get_by_label(&t("compare.export.title.report"));
        let inside = h.query_all_by_role(Role::CheckBox).count();
        assert_eq!(inside - outside, 4, "3 pairs + the paths option");
        assert!(shown(&h, &t("compare.export.warning.report_keys")));
        assert!(shown(&h, &t("compare.export.include_paths_note")));
        // Untick the second pair, then the paths option.
        let second = tw(
            "compare.pairs.pair_label",
            &[("source", "crm.invoices"), ("target", "crm.invoices")],
        );
        let boxes = h.query_all_by_role_and_label(Role::CheckBox, &second);
        assert!(boxes.count() >= 1, "a box for the pair");
        click_role(&mut h, Role::CheckBox, &second);
        assert!(matches!(
            actions(&h).last(),
            Some(CompareAction::ToggleExportPair(1))
        ));
        click_role(&mut h, Role::CheckBox, &t("compare.export.include_paths"));
        assert_eq!(
            actions(&h).last(),
            Some(&CompareAction::SetExportIncludePaths(true))
        );
    }

    #[test]
    fn the_report_modal_has_no_plan_options() {
        let h = harness(report_modal());
        assert!(!shown(&h, &t("compare.apply.id_policy.label")));
        assert!(
            h.query_by_role_and_label(Role::RadioButton, &t("compare.export.format.json"))
                .is_none()
        );
    }

    #[test]
    fn copy_emits_copy_report_and_choose_emits_the_dialog_action() {
        let mut h = harness(report_modal());
        click(&mut h, &t("compare.export.copy"));
        assert_eq!(actions(&h), vec![CompareAction::CopyReport]);

        let mut h = harness(report_modal());
        click(&mut h, &t("compare.export.choose_file"));
        assert_eq!(actions(&h), vec![CompareAction::ChooseExportFile]);

        let mut h = harness(report_modal());
        click(&mut h, &t("compare.export.cancel"));
        assert_eq!(actions(&h), vec![CompareAction::CloseExport]);
    }

    #[test]
    fn copy_is_disabled_over_the_cap_with_its_reason() {
        let mut h = harness(report_modal_with_rows(10_001));
        assert!(disabled(&h, &t("compare.export.copy")));
        assert!(shown(&h, &reason(ExportBlock::ClipboardTooLarge)));
        assert!(!disabled(&h, &t("compare.export.choose_file")));
        h.get_by_role_and_label(Role::Button, &t("compare.export.copy"))
            .click();
        h.run_steps(5);
        assert!(actions(&h).is_empty());

        let h = harness(report_modal_with_rows(10_000));
        assert!(!disabled(&h, &t("compare.export.copy")));
    }

    #[test]
    fn with_no_pair_ticked_both_outputs_are_disabled_with_a_reason() {
        let state = act(report_modal(), CompareAction::ToggleExportPair(0));
        let h = harness(state);
        assert!(disabled(&h, &t("compare.export.choose_file")));
        assert!(disabled(&h, &t("compare.export.copy")));
        assert!(shown(&h, &reason(ExportBlock::NoPairChosen)));
    }

    // ---- waiting, running, results ----

    #[test]
    fn choosing_shows_the_waiting_text_and_no_buttons() {
        let h = harness(choosing(plan_modal()));
        h.get_by_label(&t("compare.export.choosing.waiting"));
        for key in [
            "compare.export.choose_file",
            "compare.export.copy",
            "compare.export.cancel",
            "compare.export.running.cancel",
            "compare.export.dismiss",
        ] {
            assert!(!has_button(&h, key), "{key} must not be offered");
        }
        let h = harness(choosing(report_modal()));
        h.get_by_label(&t("compare.export.choosing.waiting"));
    }

    #[test]
    fn running_shows_progress_and_a_cancel_without_close() {
        let state = progressed(running_plan(), 3, 8);
        let mut h = harness(state);
        h.get_by_label(&tw(
            "compare.export.running.progress",
            &[("done", "3"), ("total", "8")],
        ));
        assert!(!has_button(&h, "compare.export.dismiss"));
        assert!(!has_button(&h, "compare.export.cancel"));
        click(&mut h, &t("compare.export.running.cancel"));
        assert_eq!(actions(&h), vec![CompareAction::CancelExport]);
        // Cancelling: the button is spent and says so.
        assert!(disabled(&h, &t("compare.export.running.cancel")));
        h.get_by_label(&t("compare.export.running.cancelling"));
        h.get_by_role_and_label(Role::Button, &t("compare.export.running.cancel"))
            .click();
        h.run_steps(5);
        assert_eq!(actions(&h).len(), 1, "cancel is sent once");
    }

    #[test]
    fn running_with_an_unknown_total_is_safe() {
        let h = harness(running_plan());
        h.get_by_label(&tw(
            "compare.export.running.progress",
            &[("done", "0"), ("total", "0")],
        ));
        let h = harness(running_clipboard());
        h.get_by_label(&tw(
            "compare.export.running.progress",
            &[("done", "0"), ("total", "0")],
        ));
    }

    fn plan_outcome() -> ExportOutcome {
        ExportOutcome {
            written: 5,
            skipped: 2,
            failed: 1,
            not_run: 3,
            bytes: 1234,
            ..ExportOutcome::default()
        }
    }

    fn count_line(key: &str, count: u64) -> String {
        tw(key, &[("count", &count.to_string())])
    }

    #[test]
    fn a_finished_plan_shows_counts_that_add_up_and_the_path() {
        let mut h = harness(finished_with(running_plan(), plan_outcome()));
        h.get_by_label(&t("compare.export.finished.title"));
        h.get_by_label(&count_line("compare.export.finished.written", 5));
        h.get_by_label(&count_line("compare.export.finished.skipped", 2));
        h.get_by_label(&count_line("compare.export.finished.failed", 1));
        h.get_by_label(&count_line("compare.export.finished.not_run", 3));
        h.get_by_label(&count_line("compare.export.finished.bytes", 1234));
        h.get_by_label(&out_path().display().to_string());
        assert!(!shown(&h, &t("compare.export.finished.partial_removed")));
        assert!(!shown(&h, &t("compare.export.finished.file_left")));
        click(&mut h, &t("compare.export.dismiss"));
        assert_eq!(actions(&h), vec![CompareAction::CloseExport]);
    }

    #[test]
    fn a_cancelled_export_says_so_and_that_the_partial_file_is_gone() {
        let outcome = ExportOutcome {
            cancelled: true,
            partial_removed: true,
            not_run: 4,
            ..ExportOutcome::default()
        };
        let h = harness(finished_with(running_plan(), outcome));
        h.get_by_label(&t("compare.export.finished.cancelled"));
        h.get_by_label(&t("compare.export.finished.partial_removed"));
        assert!(!shown(&h, &t("compare.export.finished.file_left")));
    }

    #[test]
    fn a_file_left_behind_is_a_prominent_warning_with_the_path() {
        let outcome = ExportOutcome {
            cancelled: true,
            file_left: true,
            ..ExportOutcome::default()
        };
        let h = harness(finished_with(running_plan(), outcome));
        h.get_by_label(&t("compare.export.finished.file_left"));
        h.get_by_label(&out_path().display().to_string());
        assert!(!shown(&h, &t("compare.export.finished.partial_removed")));
    }

    #[test]
    fn a_finished_report_explains_what_the_counts_mean() {
        let outcome = ExportOutcome {
            written: 10,
            skipped: 2,
            failed: 1,
            bytes: 900,
            ..ExportOutcome::default()
        };
        let h = harness(finished_with(running_report(), outcome));
        h.get_by_label(&count_line("compare.export.finished.report_rows", 10));
        h.get_by_label(&count_line("compare.export.finished.report_stale", 2));
        h.get_by_label(&count_line("compare.export.finished.report_unreadable", 1));
        assert!(!shown(
            &h,
            &count_line("compare.export.finished.written", 10)
        ));
    }

    #[test]
    fn copied_shows_the_rows_and_dismisses() {
        let mut h = harness(copied(42));
        h.get_by_label(&count_line("compare.export.copied.rows", 42));
        click(&mut h, &t("compare.export.dismiss"));
        assert_eq!(actions(&h), vec![CompareAction::CloseExport]);
    }

    #[test]
    fn running_a_chosen_export_is_reachable_only_through_the_path_answer() {
        // The view never emits ExportPathChosen: the app answers the dialog.
        let mut h = harness(plan_modal());
        click(&mut h, &t("compare.export.choose_file"));
        assert!(
            !actions(&h)
                .iter()
                .any(|a| matches!(a, CompareAction::ExportPathChosen(_)))
        );
    }
}
