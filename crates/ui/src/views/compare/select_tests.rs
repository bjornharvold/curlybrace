use super::select::show_selection_bar;
use crate::I18n;
use crate::theme::install;
use crate::viewmodel::compare::{ActionBlock, ActionButton, RowAction, SelectionBar};
use eframe::egui::{self, accesskit::Role};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

const ACTIONS: [RowAction; 4] = [
    RowAction::CopyToTarget,
    RowAction::CopyToSource,
    RowAction::DeleteFromSource,
    RowAction::DeleteFromTarget,
];

/// A bar whose buttons are all closed (a run is busy) except none: disabled buttons are the ones
/// that went through `add_enabled_ui` and were cut.
fn closed_bar() -> SelectionBar {
    SelectionBar {
        selected: 3,
        total: 40,
        tick_block: Some(ActionBlock::Busy),
        all_selected: false,
        actions: ACTIONS
            .into_iter()
            .map(|action| ActionButton {
                action,
                block: Some(ActionBlock::Busy),
                read_only: false,
            })
            .collect(),
    }
}

fn bar_at(width: f32) -> Harness<'static> {
    let bar = closed_bar();
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(width, 300.0))
        .build_ui(move |ui| {
            if !installed {
                install(ui.ctx());
                ui.ctx().set_theme(egui::Theme::Light);
                installed = true;
            }
            let _ = show_selection_bar(ui, &bar, &I18n::english());
        });
    harness.run_steps(6);
    harness
}

#[test]
fn a_wrapping_selection_bar_keeps_its_disabled_buttons_whole() {
    let i18n = I18n::english();
    let wide = bar_at(2400.0);
    let narrow = bar_at(400.0);
    let mut tops = Vec::new();
    for action in ACTIONS {
        let label = i18n.t(action.i18n_key());
        let natural = wide.get_by_role_and_label(Role::Button, &label).rect();
        let cut = narrow.get_by_role_and_label(Role::Button, &label).rect();
        assert!(
            (natural.width() - cut.width()).abs() < 0.5,
            "{label} was cut from {} to {}",
            natural.width(),
            cut.width()
        );
        assert!(cut.right() <= 400.0, "{label} runs past the row: {cut:?}");
        tops.push(cut.top());
    }
    assert!(
        tops.iter().any(|t| *t > tops[0] + 1.0),
        "the bar did not wrap"
    );
}

#[test]
fn a_closed_selection_button_is_disabled_and_explains_itself_on_hover() {
    let i18n = I18n::english();
    let mut harness = bar_at(2400.0);
    let label = i18n.t(RowAction::CopyToTarget.i18n_key());
    let reason = i18n.t(ActionBlock::Busy.i18n_key());
    use egui_kittest::kittest::NodeT;
    assert!(
        harness
            .get_by_role_and_label(Role::Button, &label)
            .accesskit_node()
            .is_disabled()
    );
    harness.get_by_role_and_label(Role::Button, &label).hover();
    harness.run_steps(12);
    harness.get_by_label(&reason);
}
