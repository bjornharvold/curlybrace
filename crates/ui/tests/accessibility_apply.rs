//! Selecting differences and queueing copy / delete actions: the row and header checkboxes, the
//! selection bar, the per-category action buttons with their disabled reasons, the detail
//! actions and the "applied" note. Fast, offline, no GPU.
mod support;

mod compare_apply_selection {
    use super::support::compare_apply_states::{
        DIFFERENT, act, applied, applying, check_out, finished, in_category, key, run_active,
        with_detail,
    };
    use curlybrace_core::compare::results::Category;
    use curlybrace_ui::I18n;
    use curlybrace_ui::app::compare::CompareAction;
    use curlybrace_ui::app::state::AppState;
    use curlybrace_ui::viewmodel::compare::{CompareState, RowAction};
    use curlybrace_ui::views::compare::show_compare;
    use curlybrace_ui::views::connections::ConnectionsState;
    use eframe::egui;
    use eframe::egui::accesskit::Role;
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    type Shown = (CompareState, ConnectionsState, Option<CompareAction>);

    fn harness(state: AppState) -> Harness<'static, Shown> {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1100.0, 1400.0))
            .build_ui_state(
                |ui, (s, c, out): &mut Shown| {
                    *out = show_compare(ui, s, c, &I18n::english()).or(out.take());
                },
                (state.compare, state.conn, None),
            );
        harness.run();
        harness
    }

    fn t(key: &str) -> String {
        I18n::english().t(key)
    }

    fn disabled(harness: &Harness<'_, Shown>, label: &str) -> bool {
        harness
            .get_by_role_and_label(Role::Button, label)
            .accesskit_node()
            .is_disabled()
    }

    fn action_label(action: RowAction) -> String {
        t(action.i18n_key())
    }

    fn present(harness: &Harness<'_, Shown>, action: RowAction) -> bool {
        harness
            .query_by_role_and_label(Role::Button, &action_label(action))
            .is_some()
    }

    const ALL: [RowAction; 4] = [
        RowAction::CopyToTarget,
        RowAction::CopyToSource,
        RowAction::DeleteFromSource,
        RowAction::DeleteFromTarget,
    ];

    fn ticked(state: AppState, n: i32) -> AppState {
        act(state, CompareAction::ToggleRow(key(n)))
    }

    #[test]
    fn every_row_has_a_checkbox_named_by_its_key_that_toggles_it() {
        let mut harness = harness(finished());
        for n in 1..=DIFFERENT as i32 {
            harness.get_by_role_and_label(Role::CheckBox, &n.to_string());
        }
        harness.get_by_role_and_label(Role::CheckBox, "2").click();
        harness.run();
        assert_eq!(harness.state().2, Some(CompareAction::ToggleRow(key(2))));
    }

    #[test]
    fn a_ticked_row_shows_its_checkbox_checked() {
        let harness = harness(ticked(finished(), 2));
        let node = harness.get_by_role_and_label(Role::CheckBox, "2");
        assert_eq!(
            node.accesskit_node().toggled(),
            Some(egui::accesskit::Toggled::True)
        );
        let other = harness.get_by_role_and_label(Role::CheckBox, "1");
        assert_eq!(
            other.accesskit_node().toggled(),
            Some(egui::accesskit::Toggled::False)
        );
        harness
            .get_by_label(&I18n::english().t_with("compare.results.selected", &[("count", "1")]));
    }

    #[test]
    fn clicking_the_key_text_still_opens_the_detail() {
        let mut harness = harness(finished());
        let row = harness
            .get_all_by_label("2")
            .find(|n| n.accesskit_node().role() != Role::CheckBox)
            .expect("the row itself");
        row.click();
        harness.run();
        assert_eq!(harness.state().2, Some(CompareAction::Select(key(2))));
    }

    #[test]
    fn the_header_checkbox_selects_the_loaded_page() {
        let mut harness = harness(finished());
        harness
            .get_by_role_and_label(Role::CheckBox, &t("compare.results.select_page"))
            .click();
        harness.run();
        assert_eq!(harness.state().2, Some(CompareAction::SelectPage));
    }

    #[test]
    fn select_all_names_the_total_of_the_tab_and_emits_its_action() {
        let mut harness = harness(finished());
        let label = I18n::english().t_with(
            "compare.results.select_all",
            &[("count", &DIFFERENT.to_string())],
        );
        harness.get_by_role_and_label(Role::Button, &label).click();
        harness.run();
        assert_eq!(harness.state().2, Some(CompareAction::SelectAllInCategory));
    }

    #[test]
    fn after_select_all_the_bar_counts_the_whole_tab() {
        let state = act(finished(), CompareAction::SelectAllInCategory);
        let harness = harness(state);
        let selected = I18n::english().t_with(
            "compare.results.selected",
            &[("count", &DIFFERENT.to_string())],
        );
        harness.get_by_label(&selected);
        assert!(
            harness
                .get_by_role_and_label(Role::CheckBox, "1")
                .accesskit_node()
                .toggled()
                == Some(egui::accesskit::Toggled::True)
        );
    }

    #[test]
    fn clear_is_disabled_until_something_is_selected_and_then_emits_clear() {
        let clear = t("compare.results.clear");
        assert!(disabled(&harness(finished()), &clear));
        let mut selected = harness(ticked(finished(), 1));
        assert!(!disabled(&selected, &clear));
        selected.get_by_role_and_label(Role::Button, &clear).click();
        selected.run();
        assert_eq!(selected.state().2, Some(CompareAction::ClearSelection));
    }

    #[test]
    fn buttons_are_offered_exactly_per_category() {
        let cases = [
            (
                Category::OnlyInSource,
                [RowAction::CopyToTarget, RowAction::DeleteFromSource],
            ),
            (
                Category::OnlyInTarget,
                [RowAction::CopyToSource, RowAction::DeleteFromTarget],
            ),
            (
                Category::Different,
                [RowAction::CopyToTarget, RowAction::CopyToSource],
            ),
        ];
        for (category, offered) in cases {
            let harness = harness(in_category(category));
            for action in ALL {
                assert_eq!(
                    present(&harness, action),
                    offered.contains(&action),
                    "{category:?} {action:?}"
                );
            }
        }
    }

    #[test]
    fn actions_are_disabled_with_nothing_selected_and_enabled_with_a_selection() {
        let none = harness(in_category(Category::OnlyInSource));
        for action in [RowAction::CopyToTarget, RowAction::DeleteFromSource] {
            assert!(disabled(&none, &action_label(action)), "{action:?}");
        }
        let mut some = harness(ticked(in_category(Category::OnlyInSource), 1));
        let delete = action_label(RowAction::DeleteFromSource);
        assert!(!disabled(&some, &delete));
        some.get_by_role_and_label(Role::Button, &delete).click();
        some.run();
        assert_eq!(
            some.state().2,
            Some(CompareAction::AddToPending(RowAction::DeleteFromSource))
        );
    }

    #[test]
    fn a_disabled_action_explains_itself_on_hover() {
        let mut none = harness(finished());
        let copy = action_label(RowAction::CopyToTarget);
        none.get_by_role_and_label(Role::Button, &copy).hover();
        none.run_steps(30);
        none.get_by_label(&t("compare.apply.reason.nothing_selected"));
    }

    #[test]
    fn an_applied_pair_says_so_and_closes_ticking_and_actions() {
        let mut harness = harness(applied(ticked(finished(), 1)));
        harness.run();
        harness.get_by_label(&t("compare.apply.applied_note"));
        assert!(
            harness
                .get_by_role_and_label(Role::CheckBox, "1")
                .accesskit_node()
                .is_disabled()
        );
        assert!(disabled(&harness, &action_label(RowAction::CopyToTarget)));
        assert!(disabled(&harness, &t("compare.results.clear")));
    }

    #[test]
    fn the_header_checkbox_clears_when_the_whole_page_is_ticked_and_selects_otherwise() {
        // Not ticked: select the page.
        let mut unticked = harness(finished());
        unticked
            .get_by_role_and_label(Role::CheckBox, &t("compare.results.select_page"))
            .click();
        unticked.run();
        assert_eq!(unticked.state().2, Some(CompareAction::SelectPage));

        // Partly ticked: still select the page.
        let mut partly = harness(ticked(finished(), 1));
        partly
            .get_by_role_and_label(Role::CheckBox, &t("compare.results.select_page"))
            .click();
        partly.run();
        assert_eq!(partly.state().2, Some(CompareAction::SelectPage));

        // Whole page ticked: the box clears the tab, and its name says so.
        let state = act(finished(), CompareAction::SelectPage);
        let mut all = harness(state);
        all.get_by_role_and_label(Role::CheckBox, &t("compare.results.clear_selection_hint"))
            .click();
        all.run();
        assert_eq!(all.state().2, Some(CompareAction::ClearSelection));
    }

    #[test]
    fn a_running_compare_or_check_disables_actions_with_their_reason() {
        let copy = action_label(RowAction::CopyToTarget);
        let mut running = harness(run_active(ticked(finished(), 1)));
        assert!(disabled(&running, &copy));
        running.get_by_role_and_label(Role::Button, &copy).hover();
        running.run_steps(30);
        running.get_by_label(&t("compare.apply.reason.running"));

        let mut checking = harness(check_out(ticked(finished(), 1)));
        checking.run_steps(2);
        assert!(disabled(&checking, &copy));
        checking.get_by_role_and_label(Role::Button, &copy).hover();
        checking.run_steps(30);
        checking.get_by_label(&t("compare.apply.reason.checking"));
    }

    #[test]
    fn an_apply_in_progress_closes_ticking_and_actions() {
        let harness = harness(applying(ticked(finished(), 1)));
        assert!(disabled(&harness, &action_label(RowAction::CopyToTarget)));
        assert!(
            harness
                .get_by_role_and_label(Role::CheckBox, "2")
                .accesskit_node()
                .is_disabled()
        );
    }

    #[test]
    fn the_pair_header_shows_a_pending_badge_with_the_count() {
        let state = ticked(ticked(finished(), 1), 3);
        let state = act(state, CompareAction::AddToPending(RowAction::CopyToTarget));
        let harness = harness(state);
        harness.get_by_label(
            &I18n::english().t_with("compare.apply.pending_badge", &[("count", "2")]),
        );
    }

    #[test]
    fn detail_actions_queue_the_one_key() {
        let mut harness = harness(with_detail(finished()));
        for action in [RowAction::CopyToTarget, RowAction::CopyToSource] {
            assert!(
                harness
                    .query_all_by_role_and_label(Role::Button, &action_label(action))
                    .count()
                    >= 1
            );
        }
        // The detail's own button is the last one drawn (below the table's bar).
        let label = action_label(RowAction::CopyToSource);
        let copy = harness
            .get_all_by_role_and_label(Role::Button, &label)
            .last()
            .expect("detail button");
        assert!(
            !copy.accesskit_node().is_disabled(),
            "a key is always chosen"
        );
        copy.click();
        harness.run();
        assert_eq!(
            harness.state().2,
            Some(CompareAction::AddKeyToPending(
                key(1),
                RowAction::CopyToSource
            ))
        );
    }

    #[test]
    fn detail_actions_are_closed_for_an_applied_pair() {
        let harness = harness(applied(with_detail(finished())));
        assert!(
            harness
                .get_all_by_role_and_label(Role::Button, &action_label(RowAction::CopyToSource))
                .all(|b| b.accesskit_node().is_disabled())
        );
    }
}
