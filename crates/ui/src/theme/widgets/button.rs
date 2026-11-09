//! Material 3 common buttons: filled (primary action), tonal (secondary) and text (low
//! emphasis). Fully rounded, 40 dp tall, label in Roboto Medium; long labels are elided.
use super::{
    CONTROL_HEIGHT, Look, Ring, disabled_container, disabled_content, focus_ring, medium_font,
    painter,
};
use crate::theme::icons;
use crate::theme::shape::{self, radius};
use crate::theme::{Scheme, current_ui};
use eframe::egui::{
    Color32, FontSelection, Id, Response, Sense, TextStyle, TextWrapMode, Ui, Vec2, WidgetInfo,
    WidgetText, WidgetType,
};

#[derive(Clone, Copy)]
enum Kind {
    Filled,
    Tonal,
    Text,
    /// A text button with snug side padding, for a crowded row.
    TextCompact,
}

impl Kind {
    /// Horizontal padding on each side of the label (Material: 24 dp, text buttons 12 dp).
    fn padding(self) -> f32 {
        match self {
            Self::Filled | Self::Tonal => shape::SPACE_6,
            Self::Text => shape::SPACE_3,
            Self::TextCompact => shape::SPACE_2,
        }
    }

    /// (container, content) when enabled.
    fn colors(self, s: &Scheme) -> (Color32, Color32) {
        match self {
            Self::Filled => (s.primary, s.on_primary),
            Self::Tonal => (s.secondary_container, s.on_secondary_container),
            Self::Text | Self::TextCompact => (Color32::TRANSPARENT, s.primary),
        }
    }
}

/// The filled button: `primary` container, `on_primary` label. For the main action.
pub fn filled_button(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    labeled(ui, text.into(), None, Kind::Filled)
}

/// The tonal button: `secondary_container` container. For secondary actions.
pub fn tonal_button(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    labeled(ui, text.into(), None, Kind::Tonal)
}

/// The text button: no container, `primary` label. For the least prominent actions.
pub fn text_button(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    labeled(ui, text.into(), None, Kind::Text)
}

/// A [`text_button`] with 8 dp instead of 12 dp of side padding, for a crowded row such as a
/// sidebar header.
pub fn compact_text_button(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    labeled(ui, text.into(), None, Kind::TextCompact)
}

/// A [`text_button`] that leads with the icon-font glyph `icon` before the plain `label`, e.g. a
/// sort arrow. The icon comes first so a narrow button elides the label, never the icon. The
/// accessible name and the elision tooltip are the plain `label`: the glyph is a private-use
/// codepoint that screen readers cannot speak and the text fonts cannot draw.
pub fn text_button_with_icon(ui: &mut Ui, icon: char, label: &str) -> Response {
    let shown = icons::icon_then_text(ui, icon, label);
    labeled(ui, shown, Some(label), Kind::Text)
}

/// Marks the theme buttons drawn by [`gate`] as off.
const GATE: &str = "theme_button_gate";

/// Draws the theme button `add` makes as enabled or, when `enabled` is false, in Material's
/// disabled look, reported disabled and without click. Unlike `Ui::add_enabled_ui` it opens no
/// child `Ui`: a child sees only the room left on the current line and a wrapping parent
/// never gets to move the button to the next one, so the button was cut to the remainder.
pub fn gate(ui: &mut Ui, enabled: bool, add: impl FnOnce(&mut Ui) -> Response) -> Response {
    let id = Id::new(GATE);
    let outer_off = ui.data(|d| d.get_temp::<bool>(id));
    // An enabled gate inside a disabled one stays disabled: it never lifts the outer state.
    let off = !enabled || outer_off.unwrap_or(false);
    ui.data_mut(|d| d.insert_temp(id, off));
    let response = add(ui);
    ui.data_mut(|d| match outer_off {
        Some(outer) => d.insert_temp(id, outer),
        None => d.remove::<bool>(id),
    });
    response
}

/// `text` as a button of `kind`; `plain` is its accessible name and tooltip when that differs
/// from the drawn text (it defaults to the drawn text).
fn labeled(ui: &mut Ui, text: WidgetText, plain: Option<&str>, kind: Kind) -> Response {
    let off = ui
        .data(|d| d.get_temp::<bool>(Id::new(GATE)))
        .unwrap_or(false);
    let usable = ui.is_enabled() && !off;
    let s = current_ui(ui);
    let padding = kind.padding();
    let size = TextStyle::Button.resolve(ui.style()).size;
    let font = FontSelection::FontId(medium_font(ui, size));
    let max_text = ui.available_width() - 2.0 * padding;
    let label = plain.map_or_else(|| text.text().to_owned(), str::to_owned);
    let galley = text.into_galley(ui, Some(TextWrapMode::Truncate), max_text, font);
    let desired = Vec2::new(galley.size().x + 2.0 * padding, CONTROL_HEIGHT);
    let (rect, mut response) = ui.allocate_exact_size(
        desired,
        if usable {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, usable, &label));
    if galley.elided {
        response = response.on_hover_text(&label);
    }
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let look = match usable {
        true => Look::of(ui, &response),
        false => Look {
            layer: None,
            focused: false,
            enabled: false,
        },
    };
    let (container, content) = kind.colors(s);
    let (fill, text_color) = if !look.enabled {
        let fill = match kind {
            Kind::Text | Kind::TextCompact => Color32::TRANSPARENT,
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
    focus_ring(&painter, rect, corners, look, s.primary, Ring::Outside);
    response
}
