//! Material 3 primary tabs: a row of text tabs over a thin divider, the selected one in
//! `primary` Roboto Medium with a 3 dp `primary` indicator, the others in `on_surface_variant`.
use super::{
    CONTROL_HEIGHT, Look, Ring, disabled_content, focus_ring, medium_font, one_row, painter,
};
use crate::theme::current_ui;
use crate::theme::shape::{self, radius};
use eframe::egui::{
    FontId, Rect, Sense, Stroke, TextStyle, Ui, WidgetInfo, WidgetType, pos2, vec2,
};

/// Height of the selected tab's indicator.
pub(crate) const INDICATOR_HEIGHT: f32 = 3.0;
/// Rounding of the indicator's top corners.
const INDICATOR_RADIUS: u8 = 2;

/// The tabs left to right over a divider that spans the available width. Each tab is reported
/// like `selectable_label` (a toggled button) with its selected state. Returns the tab clicked
/// this frame when it differs from `selected`.
pub fn tabs<T: PartialEq + Copy>(ui: &mut Ui, items: &[(T, String)], selected: T) -> Option<T> {
    tabs_row(ui, items, selected, RowWidth::Fill)
}

/// [`tabs`] that take only the width of their labels (the divider spans just the tabs), so they
/// can share a row with other widgets, e.g. after a header in a horizontal layout.
pub fn tabs_inline<T: PartialEq + Copy>(
    ui: &mut Ui,
    items: &[(T, String)],
    selected: T,
) -> Option<T> {
    tabs_row(ui, items, selected, RowWidth::Fit)
}

/// How wide the row of tabs is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RowWidth {
    /// The whole available width.
    Fill,
    /// The tabs' natural widths, at most the available width.
    Fit,
}

fn tabs_row<T: PartialEq + Copy>(
    ui: &mut Ui,
    items: &[(T, String)],
    selected: T,
    width: RowWidth,
) -> Option<T> {
    if items.is_empty() {
        return None;
    }
    let s = current_ui(ui);
    let padding = shape::SPACE_4;
    let font = medium_font(ui, TextStyle::Button.resolve(ui.style()).size);
    let available = ui.available_width();
    let row_width = match width {
        RowWidth::Fill => available,
        RowWidth::Fit => items
            .iter()
            .map(|(_, label)| natural_width(ui, label, &font, padding))
            .sum::<f32>()
            .min(available),
    };
    let base_id = ui.next_auto_id();
    let (row, _) = ui.allocate_exact_size(vec2(row_width, CONTROL_HEIGHT), Sense::hover());
    let painter = painter(ui);
    let mut left = row.left();
    let mut picked = None;
    let mut ring = None;
    for (i, (value, label)) in items.iter().enumerate() {
        let room = (row.right() - left).max(0.0);
        let width = natural_width(ui, label, &font, padding).min(room);
        let rect = Rect::from_min_size(pos2(left, row.top()), vec2(width, CONTROL_HEIGHT));
        left += width;
        let is_selected = *value == selected;
        let response = ui.interact(rect, base_id.with(i), Sense::click());
        response.widget_info(|| {
            WidgetInfo::selected(
                WidgetType::SelectableLabel,
                ui.is_enabled(),
                is_selected,
                label,
            )
        });
        let galley = one_row(ui, label, font.clone(), width - 2.0 * padding);
        let response = if galley.elided {
            response.on_hover_text(label)
        } else {
            response
        };
        let look = Look::of(ui, &response);
        let color = if !look.enabled {
            disabled_content(s.on_surface)
        } else if is_selected {
            s.primary
        } else {
            s.on_surface_variant
        };
        painter.rect_filled(rect, 0.0, look.overlay(s.on_surface));
        painter.galley(rect.center() - galley.size() / 2.0, galley, color);
        if is_selected {
            let bar = Rect::from_min_max(
                pos2(
                    rect.left() + padding / 2.0,
                    rect.bottom() - INDICATOR_HEIGHT,
                ),
                pos2(rect.right() - padding / 2.0, rect.bottom()),
            );
            let corners = eframe::egui::CornerRadius {
                nw: INDICATOR_RADIUS,
                ne: INDICATOR_RADIUS,
                sw: 0,
                se: 0,
            };
            painter.rect_filled(bar, corners, s.primary);
        }
        if look.focused {
            ring = Some((rect, look));
        }
        if response.clicked() && !is_selected {
            picked = Some(*value);
        }
    }
    // The divider under the whole row, drawn before the ring so the ring stays on top.
    let y = row.bottom() - 0.5;
    painter.hline(row.x_range(), y, Stroke::new(1.0, s.outline_variant));
    if let Some((rect, look)) = ring {
        focus_ring(&painter, rect, radius(0.0), look, s.primary, Ring::Inside);
    }
    picked
}

/// A tab's width with its label on one row and `padding` on each side.
fn natural_width(ui: &Ui, label: &str, font: &FontId, padding: f32) -> f32 {
    one_row(ui, label, font.clone(), f32::INFINITY).size().x + 2.0 * padding
}

#[cfg(test)]
#[path = "tabs_tests.rs"]
mod tests;
