//! Material 3 segmented buttons: a connected row of mutually exclusive options with pill ends,
//! an `outline` border and dividers, and the selected option on `secondary_container`.
use super::{
    COMPACT_CONTROL_HEIGHT, CONTROL_HEIGHT, Look, Ring, disabled_container, disabled_content,
    focus_ring, medium_font, one_row,
};
use crate::theme::shape::{self, radius};
use crate::theme::{Scheme, current_ui};
use eframe::egui::{
    Color32, CornerRadius, FontId, Galley, Id, Painter, Rect, Response, Sense, Stroke, StrokeKind,
    TextStyle, Ui, UiBuilder, WidgetInfo, WidgetType, pos2, vec2,
};
use std::sync::Arc;

/// Options of equal width (the widest label's, shrunk to fit the available width, labels then
/// elided). Each option is a radio button for accessibility, with its selected state. Returns
/// the option clicked this frame when it differs from `selected`.
pub fn segmented<T: PartialEq + Copy>(
    ui: &mut Ui,
    items: &[(T, String)],
    selected: T,
) -> Option<T> {
    segmented_where(ui, items, selected, |_| true)
}

/// [`segmented`] where an option for which `enabled` is false is drawn and announced disabled
/// and cannot be picked.
pub fn segmented_where<T: PartialEq + Copy>(
    ui: &mut Ui,
    items: &[(T, String)],
    selected: T,
    enabled: impl Fn(&T) -> bool,
) -> Option<T> {
    segmented_hinted(ui, items, selected, enabled, |_| None)
}

/// [`segmented_where`] where an option may carry a hover text (`hint`), shown whether the option
/// is enabled or not: a disabled option can say why it is off.
pub fn segmented_hinted<T: PartialEq + Copy>(
    ui: &mut Ui,
    items: &[(T, String)],
    selected: T,
    enabled: impl Fn(&T) -> bool,
    hint: impl Fn(&T) -> Option<String>,
) -> Option<T> {
    segmented_sized(ui, items, selected, enabled, hint, CONTROL_HEIGHT)
}

/// [`segmented_hinted`] at [`COMPACT_CONTROL_HEIGHT`], for a heading row where height is
/// scarce.
pub fn segmented_hinted_compact<T: PartialEq + Copy>(
    ui: &mut Ui,
    items: &[(T, String)],
    selected: T,
    enabled: impl Fn(&T) -> bool,
    hint: impl Fn(&T) -> Option<String>,
) -> Option<T> {
    segmented_sized(ui, items, selected, enabled, hint, COMPACT_CONTROL_HEIGHT)
}

fn segmented_sized<T: PartialEq + Copy>(
    ui: &mut Ui,
    items: &[(T, String)],
    selected: T,
    enabled: impl Fn(&T) -> bool,
    hint: impl Fn(&T) -> Option<String>,
    height: f32,
) -> Option<T> {
    if items.is_empty() {
        return None;
    }
    let s = current_ui(ui);
    let padding = shape::SPACE_3;
    let font = medium_font(ui, TextStyle::Button.resolve(ui.style()).size);
    let count = items.len() as f32;
    let item_width = item_width(ui, items, &font, padding);
    // A control that fills its row keeps the focus ring inside, clear of the parent's clip.
    let placement = if item_width * count >= ui.available_width() - 0.5 {
        Ring::Inside
    } else {
        Ring::Outside
    };
    let base_id = ui.next_auto_id();
    let (rect, _) = ui.allocate_exact_size(vec2(item_width * count, height), Sense::hover());
    let painter = super::painter(ui);
    let mut picked = None;
    let mut ring = None;
    for (i, (value, label)) in items.iter().enumerate() {
        let item_rect = Rect::from_min_size(
            pos2(rect.left() + item_width * i as f32, rect.top()),
            vec2(item_width, height),
        );
        let is_selected = *value == selected;
        let response = item_response(ui, item_rect, base_id.with(i), enabled(value));
        response.widget_info(|| {
            WidgetInfo::selected(
                WidgetType::RadioButton,
                response.enabled(),
                is_selected,
                label,
            )
        });
        let galley = one_row(ui, label, font.clone(), item_width - 2.0 * padding);
        let response = if galley.elided {
            response.on_hover_text(label)
        } else {
            response
        };
        // The hint is also the option's accessible description, so a screen reader hears why an
        // option is off without hovering it.
        let response = match hint(value) {
            Some(text) => {
                let described = text.clone();
                ui.ctx()
                    .accesskit_node_builder(response.id, |node| node.set_description(described));
                response
                    .on_hover_text(text.as_str())
                    .on_disabled_hover_text(text)
            }
            None => response,
        };
        let corners = item_corners(i, items.len());
        let look = Look::of(ui, &response);
        paint_item(&painter, s, item_rect, corners, look, is_selected, galley);
        if look.focused {
            ring = Some((item_rect, corners, look));
        }
        if response.clicked() && !is_selected {
            picked = Some(*value);
        }
    }
    paint_frame(&painter, s, rect, item_width, items.len(), ui.is_enabled());
    // Last, so the neighbouring options' fills and the dividers do not cover the ring.
    if let Some((item_rect, corners, look)) = ring {
        focus_ring(&painter, item_rect, corners, look, s.primary, placement);
    }
    picked
}

/// The click response of one option; a disabled one comes from a disabled child, so it is
/// reported (and painted) disabled and never clicked.
fn item_response(ui: &mut Ui, rect: Rect, id: Id, enabled: bool) -> Response {
    if enabled {
        return ui.interact(rect, id, Sense::click());
    }
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    child.disable();
    child.interact(rect, id, Sense::click())
}

/// The widest label plus padding, shrunk so all options fit the available width.
fn item_width<T>(ui: &Ui, items: &[(T, String)], font: &FontId, padding: f32) -> f32 {
    let widest = items
        .iter()
        .map(|(_, label)| one_row(ui, label, font.clone(), f32::INFINITY).size().x)
        .fold(0.0, f32::max);
    let fit = ui.available_width() / items.len() as f32;
    (widest + 2.0 * padding).min(fit).max(0.0)
}

/// Pill ends: the first option rounds its left corners, the last its right ones.
fn item_corners(index: usize, count: usize) -> CornerRadius {
    let full = radius(shape::FULL).nw;
    let first = index == 0;
    let last = index + 1 == count;
    CornerRadius {
        nw: if first { full } else { 0 },
        sw: if first { full } else { 0 },
        ne: if last { full } else { 0 },
        se: if last { full } else { 0 },
    }
}

fn paint_item(
    painter: &Painter,
    s: &Scheme,
    rect: Rect,
    corners: CornerRadius,
    look: Look,
    selected: bool,
    galley: Arc<Galley>,
) {
    let (fill, color) = if !look.enabled {
        let fill = if selected {
            disabled_container(s.on_surface)
        } else {
            Color32::TRANSPARENT
        };
        (fill, disabled_content(s.on_surface))
    } else if selected {
        (
            look.fill(s.secondary_container, s.on_secondary_container),
            s.on_secondary_container,
        )
    } else {
        (look.overlay(s.on_surface), s.on_surface)
    };
    painter.rect_filled(rect, corners, fill);
    painter.galley(rect.center() - galley.size() / 2.0, galley, color);
}

/// The outer border and the dividers between options.
fn paint_frame(
    painter: &Painter,
    s: &Scheme,
    rect: Rect,
    item_width: f32,
    count: usize,
    enabled: bool,
) {
    let color = if enabled {
        s.outline
    } else {
        disabled_container(s.on_surface)
    };
    let stroke = Stroke::new(1.0, color);
    painter.rect_stroke(rect, radius(shape::FULL), stroke, StrokeKind::Inside);
    for i in 1..count {
        let x = rect.left() + item_width * i as f32;
        painter.vline(x, rect.y_range(), stroke);
    }
}
