//! A sidebar row: a full-width, fully rounded pill, tonal when selected (Material's navigation
//! drawer item).
use super::{
    Look, Ring, disabled_container, disabled_content, finite_width, focus_ring, medium_font,
    painter,
};
use crate::theme::current_ui;
use crate::theme::shape::{self, radius};
use eframe::egui::{
    Color32, FontSelection, Response, Sense, TextStyle, TextWrapMode, Ui, Vec2, WidgetInfo,
    WidgetText, WidgetType, pos2,
};

/// Row height: compact enough for a tree of databases and collections.
const PILL_HEIGHT: f32 = 32.0;
/// Width used when the available width is unbounded (inside a horizontal scroll area).
const UNBOUNDED_WIDTH: f32 = 240.0;

/// A row filling the available width. Selected: `secondary_container` with
/// `on_secondary_container` text in Roboto Medium; otherwise `on_surface_variant` text on no
/// container. Long text is elided. The focus ring is drawn inside the row, since a row that
/// fills its panel has no room outside it. Reported like `selectable_label` (a toggled button).
pub fn pill(ui: &mut Ui, selected: bool, text: impl Into<WidgetText>) -> Response {
    let s = current_ui(ui);
    let text = text.into();
    let padding = shape::SPACE_4;
    let size = TextStyle::Body.resolve(ui.style()).size;
    let font = if selected {
        FontSelection::FontId(medium_font(ui, size))
    } else {
        FontSelection::Style(TextStyle::Body)
    };
    let width = finite_width(ui, UNBOUNDED_WIDTH);
    let label = text.text().to_owned();
    let galley = text.into_galley(
        ui,
        Some(TextWrapMode::Truncate),
        width - 2.0 * padding,
        font,
    );
    let desired = Vec2::new(width.max(galley.size().x + 2.0 * padding), PILL_HEIGHT);
    let (rect, mut response) = ui.allocate_exact_size(desired, Sense::click());
    response.widget_info(|| {
        WidgetInfo::selected(
            WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            &label,
        )
    });
    if galley.elided {
        response = response.on_hover_text(&label);
    }
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let look = Look::of(ui, &response);
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
        (look.overlay(s.on_surface), s.on_surface_variant)
    };
    let corners = radius(shape::FULL);
    let painter = painter(ui);
    painter.rect_filled(rect, corners, fill);
    let text_pos = pos2(
        rect.left() + padding,
        rect.center().y - galley.size().y / 2.0,
    );
    painter.galley(text_pos, galley, color);
    focus_ring(&painter, rect, corners, look, s.primary, Ring::Inside);
    response
}
