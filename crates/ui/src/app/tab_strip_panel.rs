//! The tab strip and the tab shortcuts of the Browse page (#159): the strip is drawn only when
//! more than one tab is shown (a person who never opens a second tab sees no change), the
//! shortcuts work either way.
use super::panels::UiEvent;
use super::reducer::Input;
use super::state::{AppState, Workspace};
use super::tab_actions::TabAction;
use super::tabs::TabId;
use crate::I18n;
use crate::theme::ROW_GAP;
use crate::viewmodel::tab_title::{TabLabel, labels};
use crate::views::split_panes::{SplitButtons, request_pane_focus};
use crate::views::split_shortcuts::split_shortcuts;
use crate::views::tab_shortcuts::tab_shortcuts;
use crate::views::tab_strip::{StripView, show_tab_strip};
use eframe::egui;

/// Whether a text field had the keyboard in the last frame, kept in egui's frame memory.
const TEXT_FOCUS: &str = "curlybrace.tabs.text_focus_last_frame";

/// The tab shortcuts of this frame. Read from the sessions panel, which is drawn whenever a
/// session is open, not from the Browse page: after a lost connection the page shown is the
/// connection manager, and Close and Switch must still work (only New needs a connection, and the
/// reducer refuses it).
pub(super) fn take_shortcuts(ctx: &egui::Context, state: &AppState, out: &mut Vec<UiEvent>) {
    let live = shortcuts_enabled(ctx, state);
    let wanted = tab_shortcuts(ctx, live);
    out.extend(wanted.into_iter().map(tabs_input));
    // F6 is not a text key: only a dialog holds it back.
    // (Only with a split: without one F6 belongs to whoever else wants it.)
    let panes = split_shortcuts(
        ctx,
        live,
        state.tabs.split.is_some() && no_dialog(ctx, state),
    );
    if let (Some(split), true) = (
        &state.tabs.split,
        panes.contains(&TabAction::FocusOtherPane),
    ) {
        request_pane_focus(ctx, split.secondary);
    }
    out.extend(panes.into_iter().map(tabs_input));
}

pub(super) fn show_strip(ui: &mut egui::Ui, state: &AppState, i18n: &I18n, out: &mut Vec<UiEvent>) {
    // The common case first and cheapest: one tab in all (or one shown) builds no label at all,
    // every frame.
    if state.tabs.order.len() < 2 || state.visible_tabs().len() < 2 {
        return;
    }
    let shown = labels(state, i18n);
    draw_strip(ui, state, &shown, i18n, out);
}

/// [`show_strip`] with the labels already built (the split view builds them once for the strip
/// and the panes' names).
pub(super) fn show_strip_with(
    ui: &mut egui::Ui,
    state: &AppState,
    shown: &[(TabId, TabLabel)],
    i18n: &I18n,
    out: &mut Vec<UiEvent>,
) {
    if state.tabs.order.len() < 2 || state.visible_tabs().len() < 2 {
        return;
    }
    draw_strip(ui, state, shown, i18n, out);
}

/// The strip itself, once the caller has checked there is more than one tab to show.
fn draw_strip(
    ui: &mut egui::Ui,
    state: &AppState,
    shown: &[(TabId, TabLabel)],
    i18n: &I18n,
    out: &mut Vec<UiEvent>,
) {
    let view = StripView {
        labels: shown,
        active: state.active_tab(),
        limit_reached: state.tabs.notice.is_some(),
        can_reopen: !state.tabs.closed.is_empty(),
        can_open: state.status == super::state::ConnStatus::Connected,
        split: SplitButtons {
            on: state.tabs.split.is_some(),
            orientation: state
                .tabs
                .split
                .as_ref()
                .map(|s| s.orientation)
                .unwrap_or_default(),
            single: showed_one_pane(ui, state),
        },
    };
    let actions = show_tab_strip(ui, &view, i18n);
    out.extend(actions.into_iter().map(tabs_input));
    ui.add_space(ROW_GAP);
}

/// Whether the split view showed one pane only in the last frame (the strip's orientation button
/// then changes nothing). The decision is the layout's own, made on the area the panes actually
/// get (`split_panel`, which records it): one function decides, so the strip and the panes cannot
/// disagree about the same area (the strip lags by the frame that crossed the threshold).
fn showed_one_pane(ui: &egui::Ui, state: &AppState) -> bool {
    state.tabs.split.is_some() && super::split_panel::showed_one_pane(ui.ctx())
}

/// The shortcuts act in the Browse workspace, with no dialog open (a state dialog, or any egui
/// modal: the SSH host-key question has no state) and no text field holding the keyboard (typing
/// in the Filter field is never taken). A connection is not needed: only New needs one.
fn shortcuts_enabled(ctx: &egui::Context, state: &AppState) -> bool {
    no_dialog(ctx, state) && !text_focused(ctx)
}

/// The Browse workspace with no dialog open: the keys that are not text keys work here.
fn no_dialog(ctx: &egui::Context, state: &AppState) -> bool {
    state.workspace == Workspace::Browse
        && !state.any_dialog_open()
        && ctx.memory(|m| m.top_modal_layer().is_none())
}

/// A text field has the keyboard: the focused widget is one (even scrolled out of view), or egui
/// reported an IME rectangle for one in the last frame.
fn text_focused(ctx: &egui::Context) -> bool {
    crate::views::tab_shortcuts::text_has_focus(ctx)
        || ctx
            .data(|d| d.get_temp::<bool>(egui::Id::new(TEXT_FOCUS)))
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
