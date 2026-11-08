//! A sidebar row: a full-width, fully rounded pill, tonal when selected (Material's navigation
//! drawer item).
use super::{
    Look, Ring, disabled_container, disabled_content, finite_width, focus_ring, medium_font,
    painter,
};
use crate::theme::shape::{self, radius};
use crate::theme::{current_ui, icons};
use eframe::egui::{
    Align, Color32, FontSelection, Response, RichText, Sense, TextStyle, TextWrapMode, Ui, Vec2,
    WidgetInfo, WidgetText, WidgetType, pos2, text::LayoutJob,
};

/// Row height: compact enough for a tree of databases and collections.
pub const PILL_HEIGHT: f32 = 32.0;
/// Width used when the available width is unbounded (inside a horizontal scroll area).
const UNBOUNDED_WIDTH: f32 = 240.0;

/// A row filling the available width. Selected: `secondary_container` with
/// `on_secondary_container` text in Roboto Medium; otherwise `on_surface_variant` text on no
/// container. Long text is elided. The focus ring is drawn inside the row, since a row that
/// fills its panel has no room outside it. Reported like `selectable_label` (a toggled button).
pub fn pill(ui: &mut Ui, selected: bool, text: impl Into<WidgetText>) -> Response {
    let text = text.into();
    let label = text.text().to_owned();
    show(ui, selected, text, &label, Sense::click())
}

/// A [`pill`] that can also be dragged (a sidebar row that is a drag source). `expanded` makes it
/// a disclosure row: `Some(open)` draws the matching chevron (from the icon font) before the
/// text and reports the open state to assistive technology. The accessible label is always just
/// the text: the chevron is a private-use codepoint that a screen reader would read out.
pub fn pill_row(
    ui: &mut Ui,
    selected: bool,
    expanded: Option<bool>,
    text: &str,
    sense: Sense,
) -> Response {
    let shown = match expanded {
        Some(open) => row_text(ui, selected, icons::chevron(open), text),
        None => WidgetText::from(text),
    };
    let response = show(ui, selected, shown, text, sense);
    if let Some(open) = expanded {
        ui.ctx()
            .accesskit_node_builder(response.id, |node| node.set_expanded(open));
    }
    response
}

/// The icon, a space and the text as one galley: the icon in the icon font, the text in the
/// pill's own font (Medium when selected).
fn row_text(ui: &Ui, selected: bool, icon: char, text: &str) -> WidgetText {
    let size = TextStyle::Body.resolve(ui.style()).size;
    let font = if selected {
        medium_font(ui, size)
    } else {
        TextStyle::Body.resolve(ui.style())
    };
    let style = ui.style();
    let mut job = LayoutJob::default();
    let parts = [
        icons::inline_icon(ui, icon),
        RichText::new(" "),
        RichText::new(text).font(font),
    ];
    for part in parts {
        part.append_to(&mut job, style, FontSelection::Default, Align::Center);
    }
    job.into()
}

/// Draws `text`; `label` is what assistive technology and the tooltip get instead of its glyphs.
fn show(ui: &mut Ui, selected: bool, text: WidgetText, label: &str, sense: Sense) -> Response {
    let s = current_ui(ui);
    let padding = shape::SPACE_4;
    let size = TextStyle::Body.resolve(ui.style()).size;
    let font = if selected {
        FontSelection::FontId(medium_font(ui, size))
    } else {
        FontSelection::Style(TextStyle::Body)
    };
    let width = finite_width(ui, UNBOUNDED_WIDTH);
    let galley = text.into_galley(
        ui,
        Some(TextWrapMode::Truncate),
        width - 2.0 * padding,
        font,
    );
    let desired = Vec2::new(width.max(galley.size().x + 2.0 * padding), PILL_HEIGHT);
    let (rect, mut response) = ui.allocate_exact_size(desired, sense);
    response.widget_info(|| {
        WidgetInfo::selected(
            WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            label,
        )
    });
    if galley.elided {
        response = response.on_hover_text(label);
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
