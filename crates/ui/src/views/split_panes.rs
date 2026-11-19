//! The pieces of the split view (#159, slice 3): a pane's frame, the divider between two panes
//! with its keyboard, the chip of the one-pane fallback and the strip's split buttons. Pure views:
//! what the person did comes back as `TabAction`s, and the reducer decides.
use crate::I18n;
use crate::app::tab_actions::TabAction;
use crate::app::tabs::TabId;
use crate::theme::{CONTROL_HEIGHT, current_ui, text_button};
use crate::viewmodel::split_layout::{
    DEFAULT_RATIO, DIVIDER, Orientation, PANE_RING, RATIO_STEP, clamped,
};
use eframe::egui::{
    self, Context, EventFilter, Id, Key, Layout, Modifiers, Pos2, Rect, Sense, Stroke, StrokeKind,
    Ui, UiBuilder, WidgetInfo, WidgetType, accesskit, pos2, vec2,
};

/// Where the tab a keyboard shortcut moved focus to is kept until its pane is drawn.
const FOCUS_TARGET: &str = "curlybrace.split.focus_target";

/// F6 moved focus to `tab`'s pane: its frame takes the keyboard when it is next drawn.
pub fn request_pane_focus(ctx: &Context, tab: TabId) {
    ctx.data_mut(|d| d.insert_temp(Id::new(FOCUS_TARGET), tab));
}

/// No pane is drawn (one pane fits, or no split): a pending F6 focus is dropped, so it cannot grab
/// the focus after a later resize.
pub fn clear_pane_focus(ctx: &Context) {
    ctx.data_mut(|d| d.remove::<TabId>(Id::new(FOCUS_TARGET)));
}

fn take_pane_focus(ctx: &Context, tab: TabId) -> bool {
    let wanted: Option<TabId> = ctx.data(|d| d.get_temp(Id::new(FOCUS_TARGET)));
    if wanted == Some(tab) {
        ctx.data_mut(|d| d.remove::<TabId>(Id::new(FOCUS_TARGET)));
    }
    wanted == Some(tab)
}

/// A pane: the tab it shows, where it is, whether it is the active one, and its accessible name.
pub struct PaneView<'a> {
    pub tab: TabId,
    pub rect: Rect,
    pub active: bool,
    pub name: &'a str,
    /// Draw the ring (and keep the page inside it); the one pane of the fallback has none.
    pub ring: bool,
}

/// Draws a pane: a focusable named group over `rect`, the page (`content`) inside its ring, and
/// the ring itself (the active pane's is the accent colour). The content is a child of the group
/// in the accessibility tree and every widget of it is under the pane's own id scope.
pub fn show_pane(
    ui: &mut Ui,
    pane: &PaneView<'_>,
    content: impl FnOnce(&mut Ui),
) -> egui::Response {
    let group = ui.interact(
        pane.rect,
        ui.id().with(("pane_group", pane.tab.0)),
        Sense::focusable_noninteractive(),
    );
    group.widget_info(|| WidgetInfo::labeled(WidgetType::Other, true, pane.name));
    ui.ctx().accesskit_node_builder(group.id, |node| {
        node.set_role(accesskit::Role::Group);
    });
    if take_pane_focus(ui.ctx(), pane.tab) {
        group.request_focus();
    }
    let inner = match pane.ring {
        true => pane.rect.shrink(PANE_RING),
        false => pane.rect,
    };
    let builder = UiBuilder::new()
        .id_salt(("pane", pane.tab.0))
        .max_rect(inner)
        .layout(Layout::top_down(egui::Align::Min));
    let mut child = ui.new_child(builder);
    child.set_clip_rect(inner.intersect(ui.clip_rect()));
    // Fixed ids inside the page (a field that keeps its focus) are this pane's own meanwhile.
    super::pane_scope::entered(ui.ctx(), child.id());
    ui.ctx()
        .clone()
        .with_accessibility_parent(group.id, || content(&mut child));
    super::pane_scope::left(ui.ctx());
    if pane.ring {
        paint_ring(ui, pane, group.has_focus());
    }
    group
}

fn paint_ring(ui: &Ui, pane: &PaneView<'_>, focused: bool) {
    let s = current_ui(ui);
    let (width, color) = match (pane.active, focused) {
        (_, true) => (2.0, s.primary),
        (true, false) => (2.0, s.primary),
        (false, false) => (1.0, s.outline_variant),
    };
    ui.painter().rect_stroke(
        pane.rect.shrink(width / 2.0),
        4.0,
        Stroke::new(width, color),
        StrokeKind::Middle,
    );
}

/// What the divider is drawn over: the area both panes share (so a drag can be turned into a
/// ratio) and how they are laid out.
pub struct DividerView {
    pub rect: Rect,
    pub area: Rect,
    pub orientation: Orientation,
    pub ratio: f32,
    /// The least and the most the divider can take as the panes are laid out.
    pub limits: (f32, f32),
}

/// The divider: an 8 pt bar the panes' room is shared across. A slider for the keyboard and for
/// assistive technology (named, with its value as a percentage): the arrow keys move it in 5%
/// steps, Home (or a double click) puts it back in the middle, a drag follows the pointer.
pub fn show_divider(ui: &mut Ui, view: &DividerView, i18n: &I18n) -> Option<TabAction> {
    let id = ui.id().with("split_divider");
    let response = ui.interact(view.rect, id, Sense::click_and_drag());
    let percent = (view.ratio * 100.0).round();
    let name = i18n.t("tabs.split.divider");
    let value = i18n.t_with(
        "tabs.split.divider_value",
        &[("percent", &percent.to_string())],
    );
    response.widget_info(|| WidgetInfo::slider(true, f64::from(percent), &name));
    ui.ctx().accesskit_node_builder(id, |node| {
        node.set_min_numeric_value(f64::from((view.limits.0 * 100.0).ceil()));
        node.set_max_numeric_value(f64::from((view.limits.1 * 100.0).floor()));
        node.set_numeric_value_step(RATIO_STEP as f64 * 100.0);
        node.set_value(value.clone());
    });
    paint_divider(ui, view, &response);
    let side = view.orientation == Orientation::SideBySide;
    ui.ctx()
        .set_cursor_icon(match (response.hovered() || response.dragged(), side) {
            (false, _) => egui::CursorIcon::Default,
            (true, true) => egui::CursorIcon::ResizeHorizontal,
            (true, false) => egui::CursorIcon::ResizeVertical,
        });
    let target = if response.dragged() {
        response
            .interact_pointer_pos()
            .map(|pos| ratio_at(view, pos))
    } else if response.double_clicked() {
        Some(DEFAULT_RATIO)
    } else if response.has_focus() {
        key_ratio(ui, id, view.ratio)
    } else {
        None
    };
    target
        .map(|r| clamped(r).clamp(view.limits.0, view.limits.1))
        .filter(|ratio| (ratio - view.ratio).abs() > f32::EPSILON)
        .map(TabAction::SetRatio)
}

/// The ratio that puts the divider's centre at `pos`.
fn ratio_at(view: &DividerView, pos: Pos2) -> f32 {
    let (from, along, room) = match view.orientation {
        Orientation::SideBySide => (view.area.left(), pos.x, view.area.width()),
        Orientation::Stacked => (view.area.top(), pos.y, view.area.height()),
    };
    (along - from - DIVIDER / 2.0) / (room - DIVIDER).max(1.0)
}

/// What the keys pressed with the divider focused ask for: an arrow moves it one step, Home puts
/// it in the middle. The arrows are the divider's while it has focus (egui must not move focus).
fn key_ratio(ui: &mut Ui, id: Id, ratio: f32) -> Option<f32> {
    ui.memory_mut(|m| {
        m.set_focus_lock_filter(
            id,
            EventFilter {
                horizontal_arrows: true,
                vertical_arrows: true,
                ..EventFilter::default()
            },
        );
    });
    ui.input_mut(|input| {
        let pressed = |input: &mut egui::InputState, keys: &[Key]| {
            keys.iter().fold(false, |hit, key| {
                input.consume_key(Modifiers::NONE, *key) || hit
            })
        };
        if pressed(input, &[Key::Home]) {
            Some(DEFAULT_RATIO)
        } else if pressed(input, &[Key::ArrowLeft, Key::ArrowUp]) {
            Some(ratio - RATIO_STEP)
        } else if pressed(input, &[Key::ArrowRight, Key::ArrowDown]) {
            Some(ratio + RATIO_STEP)
        } else {
            None
        }
    })
}

fn paint_divider(ui: &Ui, view: &DividerView, response: &egui::Response) {
    let s = current_ui(ui);
    let lit = response.hovered() || response.dragged() || response.has_focus();
    let ink = if lit { s.primary } else { s.outline };
    let c = view.rect.center();
    let along = view.orientation == Orientation::SideBySide;
    // A thin line along the bar, and three dots on it, so it reads as something to take hold of.
    let (from, to) = match view.orientation {
        Orientation::SideBySide => (view.rect.center_top(), view.rect.center_bottom()),
        Orientation::Stacked => (view.rect.left_center(), view.rect.right_center()),
    };
    ui.painter()
        .line_segment([from, to], Stroke::new(1.0, s.outline_variant));
    let dots_bg = if lit {
        s.primary_container
    } else {
        s.surface_container_high
    };
    let grip = match view.orientation {
        Orientation::SideBySide => vec2(6.0, 22.0),
        Orientation::Stacked => vec2(22.0, 6.0),
    };
    ui.painter().rect_filled(
        Rect::from_center_size(view.rect.center(), grip),
        3.0,
        dots_bg,
    );
    // The dots.
    for step in [-6.0, 0.0, 6.0] {
        let at = match along {
            true => c + vec2(0.0, step),
            false => c + vec2(step, 0.0),
        };
        ui.painter().circle_filled(at, 1.5, ink);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            view.rect,
            2.0,
            Stroke::new(2.0, s.primary),
            StrokeKind::Inside,
        );
    }
}

/// The chip above the one pane that is shown when two do not fit, and its button that shows the
/// other one. True when the button was pressed.
pub fn show_single_chip(ui: &mut Ui, title: &str, i18n: &I18n) -> bool {
    let text = i18n.t_with("tabs.split.single_chip", &[("title", title)]);
    ui.horizontal_wrapped(|ui| {
        ui.set_min_height(CONTROL_HEIGHT);
        ui.label(text);
        text_button(ui, i18n.t("tabs.split.single_show_other")).clicked()
    })
    .inner
}

/// How the strip's split buttons look.
#[derive(Clone, Copy, Debug, Default)]
pub struct SplitButtons {
    pub on: bool,
    pub orientation: Orientation,
    /// Only one pane fits: turning the panes would change nothing, so the button is not drawn.
    pub single: bool,
}

/// The strip's split buttons, drawn only while the view is split (a strip of an unsplit view looks
/// as it always did; the shortcut and a tab's menu split it): one that closes the split and one
/// that turns the panes the other way. Each is a square drawn from lines, named by what it does.
pub fn split_buttons(ui: &mut Ui, view: SplitButtons, i18n: &I18n) -> Vec<TabAction> {
    let mut actions = Vec::new();
    if !view.on {
        return actions;
    }
    if icon_button(ui, &i18n.t("tabs.split.off"), Glyph::Close, true) {
        actions.push(TabAction::ToggleSplit);
    }
    if view.single {
        return actions;
    }
    let (key, to, glyph) = match view.orientation {
        Orientation::SideBySide => (
            "tabs.split.orientation_stacked",
            Orientation::Stacked,
            Glyph::Stacked,
        ),
        Orientation::Stacked => (
            "tabs.split.orientation_side_by_side",
            Orientation::SideBySide,
            Glyph::Side,
        ),
    };
    if icon_button(ui, &i18n.t(key), glyph, false) {
        actions.push(TabAction::SetOrientation(to));
    }
    actions
}

/// A `CONTROL_HEIGHT` square button showing two panes laid out as `shape`.
/// What an icon button draws in its frame.
#[derive(Clone, Copy)]
enum Glyph {
    /// Two panes side by side.
    Side,
    /// Two panes one above the other.
    Stacked,
    /// A cross: the split is closed.
    Close,
}

fn icon_button(ui: &mut Ui, name: &str, shape: Glyph, pressed: bool) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(vec2(CONTROL_HEIGHT, CONTROL_HEIGHT), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, name));
    let s = current_ui(ui);
    if pressed || response.hovered() || response.has_focus() {
        ui.painter()
            .rect_filled(rect, 14.0, s.surface_container_high);
    }
    let frame = Rect::from_center_size(rect.center(), vec2(14.0, 11.0));
    let stroke = Stroke::new(1.5, s.on_surface_variant);
    ui.painter()
        .rect_stroke(frame, 2.0, stroke, StrokeKind::Middle);
    let (from, to) = match shape {
        Glyph::Side => (
            pos2(frame.center().x, frame.top()),
            pos2(frame.center().x, frame.bottom()),
        ),
        Glyph::Stacked => (
            pos2(frame.left(), frame.center().y),
            pos2(frame.right(), frame.center().y),
        ),
        Glyph::Close => (frame.left_top(), frame.right_bottom()),
    };
    ui.painter().line_segment([from, to], stroke);
    if matches!(shape, Glyph::Close) {
        ui.painter()
            .line_segment([frame.right_top(), frame.left_bottom()], stroke);
    }
    response.on_hover_text(name).clicked()
}
