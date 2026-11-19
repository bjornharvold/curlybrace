//! The tab strip and the tab shortcuts of the Browse page (#159): the strip is drawn only when
//! more than one tab is shown (a person who never opens a second tab sees no change), the
//! shortcuts work either way.
use super::panels::UiEvent;
use super::reducer::Input;
use super::state::{AppState, ConnStatus, Workspace};
use crate::I18n;
use crate::theme::ROW_GAP;
use crate::viewmodel::tab_title::labels;
use crate::views::tab_shortcuts::tab_shortcuts;
use crate::views::tab_strip::{StripView, show_tab_strip};
use eframe::egui;

/// Whether a text field had the keyboard in the last frame, kept in egui's frame memory.
const TEXT_FOCUS: &str = "curlybrace.tabs.text_focus_last_frame";

pub(super) fn show_strip(ui: &mut egui::Ui, state: &AppState, i18n: &I18n, out: &mut Vec<UiEvent>) {
    let wanted = tab_shortcuts(ui.ctx(), shortcuts_enabled(ui.ctx(), state));
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

/// The shortcuts act on the Browse page of a connected session, with no dialog open (a state
/// dialog, or any egui modal: the SSH host-key question has no state) and no text field holding
/// the keyboard (typing in the Filter field is never taken).
fn shortcuts_enabled(ctx: &egui::Context, state: &AppState) -> bool {
    state.workspace == Workspace::Browse
        && state.status == ConnStatus::Connected
        && !state.any_dialog_open()
        && ctx.memory(|m| m.top_modal_layer().is_none())
        && !text_focused(ctx)
}

fn text_focused(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(egui::Id::new(TEXT_FOCUS)))
        .unwrap_or(false)
}

/// Called once at the end of every frame, when every widget has been drawn: egui reports an IME
/// rectangle exactly when an editable text field has keyboard focus, which is the one sound way
/// to tell typing from a focused button (`wants_keyboard_input` is true for both). The next
/// frame's shortcuts read it.
pub(super) fn note_text_focus(ctx: &egui::Context) {
    let focused = ctx.output(|o| o.ime.is_some());
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(TEXT_FOCUS), focused));
}

fn tabs_input(action: super::tab_actions::TabAction) -> UiEvent {
    UiEvent::Input(Input::Tabs(action))
}
