//! Material 3 common buttons: filled (primary action), tonal (secondary) and text (low
//! emphasis). Fully rounded, 40 dp tall, label in Roboto Medium; long labels are elided.
use super::{
    CONTROL_HEIGHT, Look, disabled_container, disabled_content, focus_ring, medium_font, painter,
};
use crate::theme::shape::{self, radius};
use crate::theme::{Scheme, current_ui};
use eframe::egui::{
    Color32, FontSelection, Response, Sense, TextStyle, TextWrapMode, Ui, Vec2, WidgetInfo,
    WidgetText, WidgetType,
};

#[derive(Clone, Copy)]
enum Kind {
    Filled,
    Tonal,
    Text,
}

impl Kind {
    /// Horizontal padding on each side of the label (Material: 24 dp, text buttons 12 dp).
    fn padding(self) -> f32 {
        match self {
            Self::Filled | Self::Tonal => shape::SPACE_6,
            Self::Text => shape::SPACE_3,
        }
    }

    /// (container, content) when enabled.
    fn colors(self, s: &Scheme) -> (Color32, Color32) {
        match self {
            Self::Filled => (s.primary, s.on_primary),
            Self::Tonal => (s.secondary_container, s.on_secondary_container),
            Self::Text => (Color32::TRANSPARENT, s.primary),
        }
    }
}

/// The filled button: `primary` container, `on_primary` label. For the main action.
pub fn filled_button(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    button(ui, text.into(), Kind::Filled)
}

/// The tonal button: `secondary_container` container. For secondary actions.
pub fn tonal_button(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    button(ui, text.into(), Kind::Tonal)
}

/// The text button: no container, `primary` label. For the least prominent actions.
pub fn text_button(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    button(ui, text.into(), Kind::Text)
}

fn button(ui: &mut Ui, text: WidgetText, kind: Kind) -> Response {
    let s = current_ui(ui);
    let padding = kind.padding();
    let size = TextStyle::Button.resolve(ui.style()).size;
    let font = FontSelection::FontId(medium_font(ui, size));
    let max_text = ui.available_width() - 2.0 * padding;
    let label = text.text().to_owned();
    let galley = text.into_galley(ui, Some(TextWrapMode::Truncate), max_text, font);
    let desired = Vec2::new(galley.size().x + 2.0 * padding, CONTROL_HEIGHT);
    let (rect, mut response) = ui.allocate_exact_size(desired, Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), &label));
    if galley.elided {
        response = response.on_hover_text(&label);
    }
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let look = Look::of(ui, &response);
    let (container, content) = kind.colors(s);
    let (fill, text_color) = if !look.enabled {
        let fill = match kind {
            Kind::Text => Color32::TRANSPARENT,
            Kind::Filled | Kind::Tonal => disabled_container(s.on_surface),
        };
        (fill, disabled_content(s.on_surface))
    } else if container == Color32::TRANSPARENT {
        (look.overlay(content), content)
    } else {
        (look.fill(container, content), content)
    };
    let corners = radius(shape::FULL);
    let painter = painter(ui);
    painter.rect_filled(rect, corners, fill);
    let text_pos = rect.center() - galley.size() / 2.0;
    painter.galley(text_pos, galley, text_color);
    focus_ring(&painter, rect, corners, look, s.primary);
    response
}
