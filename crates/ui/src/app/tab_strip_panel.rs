//! The tab strip and the tab shortcuts of the Browse page (#159): the strip is drawn only when
//! more than one tab is shown (a person who never opens a second tab sees no change), the
//! shortcuts work either way.
use super::panels::UiEvent;
use super::reducer::Input;
use super::state::AppState;
use crate::I18n;
use crate::theme::ROW_GAP;
use crate::viewmodel::tab_title::labels;
use crate::views::tab_shortcuts::tab_shortcuts;
use crate::views::tab_strip::{StripView, show_tab_strip};
use eframe::egui;

pub(super) fn show_strip(ui: &mut egui::Ui, state: &AppState, i18n: &I18n, out: &mut Vec<UiEvent>) {
    let wanted = tab_shortcuts(ui.ctx(), !modal_open(state));
    out.extend(wanted.into_iter().map(tabs_input));
    let shown = labels(state, i18n);
    if shown.len() < 2 {
        return;
    }
    let view = StripView {
        labels: &shown,
        active: state.active_tab(),
        limit_reached: state.tabs.notice.is_some(),
        can_reopen: !state.tabs.closed.is_empty(),
    };
    let actions = show_tab_strip(ui, &view, i18n);
    out.extend(actions.into_iter().map(tabs_input));
    ui.add_space(ROW_GAP);
}

/// A dialog of the shown tab (or of its connection) is open: the tab shortcuts wait for it.
fn modal_open(state: &AppState) -> bool {
    state.editor.is_some()
        || state.indexes.has_prompt()
        || state.write_warning.is_some()
        || state.admin.is_some()
        || state.bulk.is_some()
        || state.copy.is_some()
        || state.aggregate.run.dialog.is_some()
}

fn tabs_input(action: super::tab_actions::TabAction) -> UiEvent {
    UiEvent::Input(Input::Tabs(action))
}
