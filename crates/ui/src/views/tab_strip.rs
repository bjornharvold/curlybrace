//! The tab strip: one row of collection tabs above the connection header (#159). The tabs scroll
//! sideways and keep the active one in view; "All tabs" and "New tab" are pinned at the right, and
//! the limit notice sits beside them. Each tab says its connection's colour, production and
//! read-only badge, a mark when it holds unsaved work, and has its own close button. Everything it
//! wants done is returned as `TabAction`s; the reducer decides.
use crate::I18n;
use crate::app::tab_actions::{MAX_TABS, TabAction};
use crate::app::tabs::TabId;
use crate::theme::{CONTROL_HEIGHT, current_ui};
use crate::viewmodel::ConnectionBadge;
use crate::viewmodel::tab_title::TabLabel;
use crate::views::connection_badge::show_badge;
use crate::views::tab_strip_menu::{all_tabs_menu, tab_context_menu};
use eframe::egui::{
    self, Align, Key, Label, Layout, Modifiers, Rect, RichText, Sense, Stroke, Ui, UiBuilder,
    WidgetInfo, WidgetType, accesskit, pos2, vec2,
};

/// A tab is at least this wide (points), so a short name still reads as a tab.
const MIN_TAB_WIDTH: f32 = 96.0;
/// A tab is at most this wide; a longer name is cut and the tooltip gives all of it.
const MAX_TAB_WIDTH: f32 = 180.0;
const PADDING: f32 = 8.0;
/// The close button's square.
const CLOSE_SIZE: f32 = 24.0;
const INDICATOR_HEIGHT: f32 = 3.0;
/// The production and read-only marks inside a tab.
const MARK_SIZE: f32 = 12.0;

/// What the strip shows.
pub struct StripView<'a> {
    pub labels: &'a [(TabId, TabLabel)],
    pub active: Option<TabId>,
    /// The last new tab was refused: say why beside the buttons.
    pub limit_reached: bool,
    /// A closed tab can be reopened.
    pub can_reopen: bool,
}

/// Draws the strip in one row of `CONTROL_HEIGHT`; returns what the person did.
pub fn show_tab_strip(ui: &mut Ui, view: &StripView<'_>, i18n: &I18n) -> Vec<TabAction> {
    let mut actions = Vec::new();
    let size = vec2(ui.available_width(), CONTROL_HEIGHT);
    ui.allocate_ui_with_layout(size, Layout::right_to_left(Align::Center), |ui| {
        actions.extend(all_tabs_menu(ui, view, i18n));
        actions.extend(new_tab_button(ui, i18n));
        if view.limit_reached {
            let max = MAX_TABS.to_string();
            let notice = ui.label(i18n.t_with("tabs.limit_reached", &[("max", &max)]));
            // A live region: a screen reader says it when it appears.
            ui.ctx().accesskit_node_builder(notice.id, |node| {
                node.set_live(accesskit::Live::Polite);
            });
        }
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            tab_list(ui, view, i18n, &mut actions);
        });
    });
    actions
}

fn tab_list(ui: &mut Ui, view: &StripView<'_>, i18n: &I18n, actions: &mut Vec<TabAction>) {
    let list = ui.interact(ui.max_rect(), ui.id().with("tab_list"), Sense::hover());
    let name = i18n.t("tabs.strip");
    list.widget_info(|| WidgetInfo::labeled(WidgetType::Other, true, &name));
    ui.ctx().accesskit_node_builder(list.id, |node| {
        node.set_role(accesskit::Role::TabList);
    });
    let scrolled_to = ui.id().with("tab_strip_scrolled_to");
    let previous: Option<TabId> = ui.data(|d| d.get_temp(scrolled_to));
    // The tabs are the list's children in the accessibility tree, not only inside its rectangle.
    ui.ctx().clone().with_accessibility_parent(list.id, || {
        egui::ScrollArea::horizontal()
            .id_salt("tab_strip_scroll")
            .auto_shrink([false, false])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.horizontal(|ui| {
                    let mut rects = Vec::new();
                    let mut dropped = None;
                    for (at, (tab, label)) in view.labels.iter().enumerate() {
                        let active = view.active == Some(*tab);
                        let shown = show_tab(ui, (at, *tab, label), active, view, i18n, actions);
                        if active && previous != Some(*tab) {
                            ui.scroll_to_rect(shown.rect, None);
                        }
                        rects.push((*tab, shown.rect));
                        dropped = dropped.or(shown.dropped_at.map(|pos| (*tab, pos)));
                    }
                    actions.extend(reordered(&rects, dropped));
                });
            });
    });
    ui.data_mut(|d| match view.active {
        Some(tab) => d.insert_temp(scrolled_to, tab),
        None => d.remove::<TabId>(scrolled_to),
    });
}

/// `Move` for a tab dropped over another tab's place.
fn reordered(rects: &[(TabId, Rect)], dropped: Option<(TabId, egui::Pos2)>) -> Option<TabAction> {
    let (tab, at) = dropped?;
    let from = rects.iter().position(|(t, _)| *t == tab)?;
    let to = rects.iter().position(|(_, r)| r.x_range().contains(at.x))?;
    (to != from).then_some(TabAction::Move { tab, to })
}

struct Shown {
    rect: Rect,
    /// Where a drag of this tab ended.
    dropped_at: Option<egui::Pos2>,
}

fn show_tab(
    ui: &mut Ui,
    (at, tab, label): (usize, TabId, &TabLabel),
    active: bool,
    view: &StripView<'_>,
    i18n: &I18n,
    actions: &mut Vec<TabAction>,
) -> Shown {
    let width = (natural_width(label)).clamp(MIN_TAB_WIDTH, MAX_TAB_WIDTH);
    let (rect, _) = ui.allocate_exact_size(vec2(width, CONTROL_HEIGHT), Sense::hover());
    let id = ui.make_persistent_id(("tab", tab.0));
    let close_rect = Rect::from_center_size(
        pos2(rect.right() - 2.0 - CLOSE_SIZE / 2.0, rect.center().y),
        vec2(CLOSE_SIZE, CLOSE_SIZE),
    );
    // The tab proper ends where its close button begins: the two never overlap.
    let body = Rect::from_min_max(rect.min, pos2(close_rect.left(), rect.max.y));
    let response = ui.interact(body, id, Sense::click_and_drag());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &label.accessible));
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_role(accesskit::Role::Tab);
        node.set_selected(active);
    });
    paint_tab(ui, rect, &response, active);
    paint_content(ui, rect, close_rect, label, active, i18n);
    let close = close_button(ui, tab, close_rect, label, i18n);
    let response = response.on_hover_text(&label.tooltip);
    if response.clicked() && !active {
        actions.push(TabAction::Activate(tab));
    }
    if close || response.middle_clicked() {
        actions.push(TabAction::Close(tab));
    }
    response.context_menu(|ui| {
        let entry = (tab, at, view.labels.len());
        actions.extend(tab_context_menu(ui, entry, view.can_reopen, i18n));
    });
    if response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    arrow_keys_activate(ui, &response, at, view, actions);
    Shown {
        rect,
        dropped_at: response
            .drag_stopped()
            .then(|| response.interact_pointer_pos())
            .flatten(),
    }
}

/// Enter or Space on a focused tab is a click (egui does that); here the arrows only move focus
/// (egui's own directional focus), and Home / End jump to the first / last tab.
fn arrow_keys_activate(
    ui: &Ui,
    response: &egui::Response,
    at: usize,
    view: &StripView<'_>,
    actions: &mut Vec<TabAction>,
) {
    if !response.has_focus() {
        return;
    }
    let target = if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Home)) {
        view.labels.first()
    } else if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::End)) {
        view.labels.last()
    } else {
        None
    };
    if let Some((tab, _)) = target.filter(|_| at < view.labels.len()) {
        actions.push(TabAction::Activate(*tab));
        ui.memory_mut(|m| m.request_focus(ui.make_persistent_id(("tab", tab.0))));
    }
}

/// The room a tab's content wants, before it is held to its limits.
fn natural_width(label: &TabLabel) -> f32 {
    let badges = [
        label.badge.color.is_some().then_some(18.0),
        label.badge.production.then_some(MARK_SIZE + 4.0),
        label.badge.read_only.then_some(MARK_SIZE + 4.0),
    ];
    let chars = label.title.chars().count() as f32;
    2.0 * PADDING + CLOSE_SIZE + 6.0 + chars * 7.0 + badges.into_iter().flatten().sum::<f32>()
}

fn paint_tab(ui: &Ui, rect: Rect, response: &egui::Response, active: bool) {
    let s = current_ui(ui);
    let fill = match (active, response.hovered()) {
        (true, _) => s.surface_container_high,
        (false, true) => s.surface_container,
        (false, false) => s.surface,
    };
    let top = egui::CornerRadius {
        nw: 6,
        ne: 6,
        sw: 0,
        se: 0,
    };
    let painter = ui.painter();
    painter.rect_filled(rect, top, fill);
    if active {
        let bar = Rect::from_min_max(
            pos2(
                rect.left() + PADDING / 2.0,
                rect.bottom() - INDICATOR_HEIGHT,
            ),
            pos2(rect.right() - PADDING / 2.0, rect.bottom()),
        );
        painter.rect_filled(bar, 2.0, s.primary);
    }
    if response.has_focus() {
        painter.rect_stroke(
            rect.shrink(1.0),
            top,
            Stroke::new(2.0, s.primary),
            egui::StrokeKind::Inside,
        );
    }
}

/// The badge, the unsaved mark and the title, in the room the close button leaves.
fn paint_content(
    ui: &mut Ui,
    rect: Rect,
    close_rect: Rect,
    label: &TabLabel,
    active: bool,
    i18n: &I18n,
) {
    let s = current_ui(ui);
    let inner = Rect::from_min_max(
        pos2(rect.left() + PADDING, rect.top()),
        pos2(
            close_rect.left() - 4.0,
            rect.bottom() - INDICATOR_HEIGHT / 2.0,
        ),
    );
    let color = if active {
        s.on_surface
    } else {
        s.on_surface_variant
    };
    let builder = UiBuilder::new()
        .max_rect(inner)
        .layout(Layout::left_to_right(Align::Center));
    // A child that is not allocated back into the row: the tab's room was taken already, and the
    // content is smaller than it (an allocation would pull the row's cursor back inside the tab).
    let mut content = ui.new_child(builder);
    content.spacing_mut().item_spacing.x = 4.0;
    // The connection's colour glyph as in the header; production and read-only are short marks
    // here (the words are in the tab's tooltip and accessible name), so the title keeps its room.
    let glyph = ConnectionBadge {
        production: false,
        read_only: false,
        ..label.badge
    };
    show_badge(&mut content, &glyph, i18n);
    if label.badge.production {
        paint_mark(&mut content, (s.error, s.on_error), Mark::Exclamation);
    }
    if label.badge.read_only {
        paint_mark(&mut content, (s.warning, s.on_warning), Mark::Bar);
    }
    if label.unsaved {
        let (dot, _) = content.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
        content
            .painter()
            .circle_filled(dot.center(), 3.0, s.primary);
    }
    // A dormant tab is drawn dimmer and slanted: it waits for Run.
    let mut title = RichText::new(&label.title).color(color);
    if label.dormant {
        title = title.italics().color(color.gamma_multiply(0.7));
    }
    content.add(Label::new(title).truncate().selectable(false));
}

/// The shape inside a compact production / read-only mark.
enum Mark {
    Exclamation,
    Bar,
}

/// A small round mark of `fill` with a white `shape` in it: not text, so nothing to translate.
fn paint_mark(ui: &mut Ui, (fill, on): (egui::Color32, egui::Color32), shape: Mark) {
    let size = MARK_SIZE;
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let painter = ui.painter();
    painter.circle_filled(rect.center(), size / 2.0, fill);
    let stroke = Stroke::new(1.5, on);
    let c = rect.center();
    match shape {
        Mark::Exclamation => {
            painter.line_segment([c + vec2(0.0, -3.5), c + vec2(0.0, 0.5)], stroke);
            painter.circle_filled(c + vec2(0.0, 3.0), 0.9, on);
        }
        Mark::Bar => {
            painter.line_segment([c + vec2(-3.0, 0.0), c + vec2(3.0, 0.0)], stroke);
        }
    }
}

/// The × button, named after its tab. A square drawn from two lines: the icon font has no cross.
fn close_button(ui: &mut Ui, tab: TabId, rect: Rect, label: &TabLabel, i18n: &I18n) -> bool {
    let id = ui.make_persistent_id(("tab_close", tab.0));
    let response = ui.interact(rect, id, Sense::click());
    let name = i18n.t_with("tabs.close_named", &[("title", &label.title)]);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &name));
    let s = current_ui(ui);
    if response.hovered() || response.has_focus() {
        ui.painter()
            .rect_filled(rect, 4.0, s.surface_container_high);
    }
    let reach = CLOSE_SIZE / 6.0;
    let stroke = Stroke::new(1.5, s.on_surface_variant);
    let c = rect.center();
    ui.painter()
        .line_segment([c + vec2(-reach, -reach), c + vec2(reach, reach)], stroke);
    ui.painter()
        .line_segment([c + vec2(-reach, reach), c + vec2(reach, -reach)], stroke);
    response.on_hover_text(name).clicked()
}

/// The + button that opens an empty tab.
fn new_tab_button(ui: &mut Ui, i18n: &I18n) -> Option<TabAction> {
    let (rect, response) =
        ui.allocate_exact_size(vec2(CONTROL_HEIGHT, CONTROL_HEIGHT), Sense::click());
    let name = i18n.t("tabs.new_tab");
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &name));
    let s = current_ui(ui);
    if response.hovered() || response.has_focus() {
        ui.painter()
            .rect_filled(rect, 14.0, s.surface_container_high);
    }
    let reach = 6.0;
    let stroke = Stroke::new(1.5, s.on_surface_variant);
    let c = rect.center();
    ui.painter()
        .line_segment([c - vec2(reach, 0.0), c + vec2(reach, 0.0)], stroke);
    ui.painter()
        .line_segment([c - vec2(0.0, reach), c + vec2(0.0, reach)], stroke);
    response
        .on_hover_text(name)
        .clicked()
        .then_some(TabAction::New)
}
