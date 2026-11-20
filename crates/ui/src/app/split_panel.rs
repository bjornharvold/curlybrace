//! Draws the split view: the strip once, then the two panes with the divider between them, or one
//! pane with a chip when two do not fit (#159, slice 3).
//!
//! The active pane is drawn as the Browse page always is. The other pane is drawn by swapping its
//! tab into the foreground for the length of its draw (the same swap the reducer routes answers
//! with, `AppState::with_tab`, which brings the tab's own connection with it): the page code reads
//! `state.page` and `state.find` and never learns that there are two. Its events leave as
//! `Input::InTab` (the person touched the pane: it becomes the active one) or `Input::ForTab` (the
//! pane acted alone, an automatic refresh: the focus stays), so they are applied in the tab and
//! the connection that drew them.
use super::browse_panel::{show_code_dialog_once, show_pane as show_page};
use super::panels::UiEvent;
use super::reducer::Input;
use super::state::{AppState, ConnStatus};
use super::tab_actions::TabAction;
use super::tab_strip::Split;
use super::tabs::{Freshness, TabId};
use crate::I18n;
use crate::theme::ROW_GAP;
use crate::viewmodel::split_layout::{DIVIDER, Orientation, SplitLayout, layout, ratio_limits};
use crate::viewmodel::tab_title::{TabLabel, labels};
use crate::views::split_panes::{
    DividerView, PaneView, clear_pane_focus, show_divider, show_pane, show_single_chip,
};
use eframe::egui::{self, Id, Rect, Sense, pos2, vec2};

/// The Browse page of the foreground, split in two when the strip says so.
pub(super) fn show_browse(
    ui: &mut egui::Ui,
    state: &mut AppState,
    i18n: &I18n,
    out: &mut Vec<UiEvent>,
    now_secs: i64,
) {
    // The labels are built once for the strip and for the panes' names.
    let shown = match state.tabs.split.is_some() {
        true => labels(state, i18n),
        false => Vec::new(),
    };
    match state.tabs.split.is_some() {
        true => super::tab_strip_panel::show_strip_with(ui, state, &shown, i18n, out),
        false => super::tab_strip_panel::show_strip(ui, state, i18n, out),
    }
    // The dialog is the app's, drawn once in every mode, before the page or the panes.
    show_code_dialog_once(ui.ctx(), state, i18n, out);
    let Some(split) = state.tabs.split.clone() else {
        clear_pane_focus(ui.ctx());
        note_single(ui.ctx(), false);
        return show_page(ui, state, i18n, out, now_secs);
    };
    let area = ui.available_rect_before_wrap();
    let names = Names::of(&shown, &split, i18n);
    let chosen = layout(area.size(), split.orientation, split.ratio);
    note_single(ui.ctx(), chosen == SplitLayout::Single);
    match chosen {
        SplitLayout::Single => show_single(ui, state, (i18n, now_secs), out, &names),
        SplitLayout::Side { ratio } => {
            let shape = (area, Orientation::SideBySide, ratio);
            show_two(ui, state, &split, (i18n, now_secs), (shape, &names), out)
        }
        SplitLayout::Stacked { ratio } => {
            let shape = (area, Orientation::Stacked, ratio);
            show_two(ui, state, &split, (i18n, now_secs), (shape, &names), out)
        }
    }
}

const SINGLE: &str = "curlybrace.split.showed_one_pane";

fn note_single(ctx: &egui::Context, single: bool) {
    ctx.data_mut(|d| d.insert_temp(Id::new(SINGLE), single));
}

/// Whether the last frame of the split view showed one pane (see `tab_strip_panel`).
pub(super) fn showed_one_pane(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(Id::new(SINGLE)))
        .unwrap_or(false)
}

/// What the two panes are called: the accessible names and the title the chip shows.
struct Names {
    active: String,
    other: String,
    title: String,
}

impl Names {
    fn of(shown: &[(TabId, TabLabel)], split: &Split, i18n: &I18n) -> Self {
        let find = |tab: TabId| shown.iter().find(|(t, _)| *t == tab).map(|(_, l)| l);
        let named = |key: &str, tab: TabId| {
            let name = find(tab).map(|l| l.accessible.as_str()).unwrap_or_default();
            i18n.t_with(key, &[("name", name)])
        };
        Self {
            active: named("tabs.split.pane_active", split.primary),
            other: named("tabs.split.pane_other", split.secondary),
            title: find(split.primary)
                .map(|l| l.title.clone())
                .unwrap_or_default(),
        }
    }
}

type Frame<'a> = (&'a I18n, i64);

/// No room for two panes: a chip that says so, with the button that shows the other one, and the
/// active pane under it as the page always is.
fn show_single(
    ui: &mut egui::Ui,
    state: &mut AppState,
    (i18n, now_secs): Frame<'_>,
    out: &mut Vec<UiEvent>,
    names: &Names,
) {
    clear_pane_focus(ui.ctx());
    if show_single_chip(ui, &names.title, i18n) {
        out.push(tabs_event(TabAction::ShowOtherPane));
    }
    ui.add_space(ROW_GAP);
    // One pane, drawn as the page always is: no group, no ring, no second tab stop.
    ui.push_id(("single", state.active_tab().map(|t| t.0)), |ui| {
        show_page(ui, state, i18n, out, now_secs)
    });
}

fn show_two(
    ui: &mut egui::Ui,
    state: &mut AppState,
    split: &Split,
    (i18n, now_secs): Frame<'_>,
    ((area, orientation, ratio), names): ((Rect, Orientation, f32), &Names),
    out: &mut Vec<UiEvent>,
) {
    let (first, bar, second) = rects(area, orientation, ratio);
    let (active_rect, other_rect) = match split.secondary_first {
        true => (second, first),
        false => (first, second),
    };
    // Drawn in the order the keyboard visits them: the active pane, the divider, the other pane.
    let active = PaneView {
        tab: split.primary,
        rect: active_rect,
        active: true,
        name: &names.active,
    };
    show_pane(ui, &active, |ui| show_page(ui, state, i18n, out, now_secs));
    let divider = DividerView {
        rect: bar,
        area,
        orientation,
        ratio,
        limits: ratio_limits(area.size(), orientation),
    };
    if let Some(action) = show_divider(ui, &divider, i18n) {
        out.push(tabs_event(action));
    }
    let other = PaneView {
        tab: split.secondary,
        rect: other_rect,
        active: false,
        name: &names.other,
    };
    show_other(ui, state, &other, (i18n, now_secs), out);
    ui.allocate_rect(area, Sense::hover());
}

/// The panes' rectangles in drawing order of the screen: first, divider, second.
fn rects(area: Rect, orientation: Orientation, ratio: f32) -> (Rect, Rect, Rect) {
    match orientation {
        Orientation::SideBySide => {
            let width = ((area.width() - DIVIDER) * ratio).round();
            let first = Rect::from_min_size(area.min, vec2(width, area.height()));
            let bar = Rect::from_min_size(first.right_top(), vec2(DIVIDER, area.height()));
            let second = Rect::from_min_max(bar.right_top(), area.max);
            (first, bar, second)
        }
        Orientation::Stacked => {
            let height = ((area.height() - DIVIDER) * ratio).round();
            let first = Rect::from_min_size(area.min, vec2(area.width(), height));
            let bar = Rect::from_min_size(first.left_bottom(), vec2(area.width(), DIVIDER));
            let second = Rect::from_min_max(pos2(area.left(), bar.bottom()), area.max);
            (first, bar, second)
        }
    }
}

/// The other pane: its tab swapped into the foreground for the draw, its events sent as `InTab`.
fn show_other(
    ui: &mut egui::Ui,
    state: &mut AppState,
    pane: &PaneView<'_>,
    (i18n, now_secs): Frame<'_>,
    out: &mut Vec<UiEvent>,
) {
    let tab = pane.tab;
    // It is on screen now: a stale pane reads again what it shows (a dormant one does not).
    if is_stale_and_reachable(state, tab) {
        out.push(tabs_event(TabAction::PaneShown(tab)));
    }
    let mut events = Vec::new();
    let taken = std::mem::take(state);
    let (back, _) = taken.with_tab(tab, |mut foreground| {
        show_pane(ui, pane, |ui| {
            show_page(ui, &mut foreground, i18n, &mut events, now_secs)
        });
        (foreground, Vec::new())
    });
    *state = back;
    // What the person did in the pane focuses it; what the pane did on its own (an automatic
    // refresh of an idle preview) is applied in its tab and leaves the focus where it is.
    let touched = focus_moved_into(ui, pane.rect);
    out.extend(events.into_iter().map(|event| in_tab(tab, touched, event)));
    if touched {
        out.push(tabs_event(TabAction::Activate(tab)));
    }
}

fn is_stale_and_reachable(state: &AppState, tab: TabId) -> bool {
    let session = state.tabs.owner.get(&tab).copied();
    state.freshness_of(tab) == Freshness::Stale
        && state.tab_ns(tab).is_some()
        && session.and_then(|s| state.status_of(s)) == Some(ConnStatus::Connected)
}

/// Focus follows interaction: a press inside `rect`, or the keyboard arriving at a widget inside
/// it, makes its pane the active one.
///
/// Only on the pane's own layer, and never behind a modal: a dialog or a popup lying over the pane
/// (an editor, a context menu) is egui's own layer, and a press on it is not a touch of the pane.
fn focus_moved_into(ui: &egui::Ui, rect: Rect) -> bool {
    if ui.ctx().memory(|m| m.top_modal_layer()).is_some() {
        let focused = ui.memory(|m| m.focused());
        ui.data_mut(|d| d.insert_temp(Id::new("curlybrace.split.last_focus"), focused));
        return false;
    }
    let own = ui.layer_id();
    let pressed = ui.input(|i| {
        let at = i.pointer.interact_pos();
        at.filter(|p| i.pointer.any_pressed() && rect.contains(*p))
    });
    let pressed = pressed.is_some_and(|p| !overlaid(ui.ctx(), p, own));
    let key = Id::new("curlybrace.split.last_focus");
    let focused = ui.memory(|m| m.focused());
    let before: Option<Option<Id>> = ui.data(|d| d.get_temp(key));
    ui.data_mut(|d| d.insert_temp(key, focused));
    let arrived = match (focused, before) {
        (Some(id), Some(last)) if last != Some(id) => ui
            .ctx()
            .read_response(id)
            .is_some_and(|r| r.layer_id == own && rect.contains(r.rect.center())),
        _ => false,
    };
    pressed || arrived
}

fn tabs_event(action: TabAction) -> UiEvent {
    UiEvent::Input(Input::Tabs(action))
}

/// What a pane sent, as the tab's own: with the focus when the person touched the pane, without
/// it when the pane acted alone.
fn in_tab(tab: TabId, touched: bool, event: UiEvent) -> UiEvent {
    let input = |input| Box::new(input);
    match (event, touched) {
        (UiEvent::Input(i), true) => UiEvent::Input(Input::InTab {
            tab,
            input: input(i),
        }),
        (UiEvent::Input(i), false) => UiEvent::Input(Input::ForTab {
            tab,
            input: input(i),
        }),
        (other, _) => other,
    }
}

/// Whether something of egui's own lies over `pos`: a popup, a menu, a tooltip or a dialog is a
/// layer above the page's (its area need not be interactable, so `layer_id_at` alone misses it).
fn overlaid(ctx: &egui::Context, pos: egui::Pos2, own: egui::LayerId) -> bool {
    ctx.layer_id_at(pos) != Some(own)
        || ctx.memory(|m| {
            m.layer_ids().any(|layer| {
                // egui never forgets a layer it has seen: only those drawn now count.
                layer.order != egui::Order::Background
                    && m.areas().is_visible(&layer)
                    && m.area_rect(layer.id).is_some_and(|r| r.contains(pos))
            })
        })
}
