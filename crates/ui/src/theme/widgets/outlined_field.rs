//! Material 3 outlined text field: a single-line `TextEdit` inside a rounded 1 dp `outline`
//! border (2 dp `primary` while focused) with its label floating in a gap on the top border.
use super::{CONTROL_HEIGHT, Look, disabled_container, disabled_content, one_row, painter};
use crate::theme::shape::{self, radius};
use crate::theme::{Scheme, current_ui};
use eframe::egui::{
    Color32, FontId, Galley, Id, Margin, Painter, Rect, Response, Sense, Stroke, StrokeKind,
    TextEdit, TextStyle, Ui, accesskit::Invalid, pos2, text_edit::TextBuffer, vec2,
};
use std::sync::Arc;

/// Space between the floating label and the cut ends of the border.
const LABEL_GAP: f32 = shape::SPACE_1;
/// Border width while focused (Material: 2 dp `primary`).
const FOCUSED_BORDER: f32 = 2.0;

/// An outlined single-line field editing `text`, filling the available width, with `hint` shown
/// while it is empty. `label` floats on the border and is the field's accessible name; the
/// value is exposed as the field's text. Returns the `TextEdit`'s response.
pub fn outlined_field(ui: &mut Ui, label: &str, text: &mut String, hint: &str) -> Response {
    outlined_field_with(ui, label, text, hint, None)
}

/// [`outlined_field`] with an optional validation `error`. With one, the border and label use
/// the scheme's `error` color, the message is shown under the field as supporting text, and
/// the field is reported invalid with the message as its accessible description.
pub fn outlined_field_with(
    ui: &mut Ui,
    label: &str,
    text: &mut String,
    hint: &str,
    error: Option<&str>,
) -> Response {
    let options = FieldOptions {
        error,
        ..FieldOptions::default()
    };
    outlined_field_opts(ui, label, text, hint, options)
}

/// What an outlined field can do beyond editing a `String`.
#[derive(Clone, Copy, Default)]
pub struct FieldOptions<'a> {
    /// A validation message, as in [`outlined_field_with`].
    pub error: Option<&'a str>,
    /// Mask the text (a secret); the field is then reported as a password input.
    pub password: bool,
    /// A fixed widget id, for a field whose editor state or focus is addressed from outside.
    pub id: Option<Id>,
    /// Keep the row of the error message free while there is none, so showing an error does not
    /// move what is below the field.
    pub reserve_error_row: bool,
}

/// [`outlined_field_with`] over any text buffer (a read-only `&str` shows selectable text) with
/// the extra [`FieldOptions`].
pub fn outlined_field_opts(
    ui: &mut Ui,
    label: &str,
    text: &mut dyn TextBuffer,
    hint: &str,
    options: FieldOptions<'_>,
) -> Response {
    let error = options.error;
    let s = current_ui(ui);
    let small = FontId::proportional(TextStyle::Small.resolve(ui.style()).size);
    let max_label = ui.available_width() - 2.0 * (shape::SPACE_3 + LABEL_GAP);
    let label_galley = one_row(ui, label, small.clone(), max_label);
    let half_label = label_galley.size().y / 2.0;
    ui.vertical(|ui| {
        // Room above the border for the upper half of the floating label.
        ui.add_space(half_label);
        let row = ui.text_style_height(&TextStyle::Body);
        let vertical = ((CONTROL_HEIGHT - row) / 2.0).max(0.0);
        let mut edit = TextEdit::singleline(text)
            .hint_text(hint)
            .frame(false)
            .margin(Margin::symmetric(shape::SPACE_4 as i8, vertical as i8))
            .desired_width(f32::INFINITY)
            .password(options.password)
            .text_color(s.on_surface);
        if let Some(id) = options.id {
            edit = edit.id(id);
        }
        let response = ui.add(edit);
        describe(ui, &response, label, hint, error);
        if ui.is_rect_visible(response.rect) {
            let look = Look::of(ui, &response);
            let (stroke, label_color) = colors(s, look, error.is_some());
            paint(
                &painter(ui),
                response.rect,
                stroke,
                label_color,
                label_galley,
            );
        }
        match (error, options.reserve_error_row) {
            (Some(message), _) => supporting_text(ui, s, message, small),
            (None, true) => reserve_supporting_row(ui, &small),
            (None, false) => {}
        }
        response
    })
    .inner
}

/// The label as the accessible name, the hint as placeholder, and an error as the description
/// of an invalid field.
fn describe(ui: &Ui, response: &Response, label: &str, hint: &str, error: Option<&str>) {
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_label(label);
        node.set_placeholder(hint);
        if let Some(message) = error {
            node.set_invalid(Invalid::True);
            node.set_description(message);
        }
    });
}

/// The height of a supporting text row, left empty.
fn reserve_supporting_row(ui: &mut Ui, font: &FontId) {
    let galley = one_row(ui, " ", font.clone(), ui.available_width());
    let height = galley.size().y + shape::SPACE_1;
    ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
}

/// One row of `message` in `error` under the field, aligned with the field's text.
fn supporting_text(ui: &mut Ui, s: &Scheme, message: &str, font: FontId) {
    let width = ui.available_width();
    let galley = one_row(ui, message, font, width - 2.0 * shape::SPACE_4);
    let height = galley.size().y + shape::SPACE_1;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    if ui.is_rect_visible(rect) {
        let color = if ui.is_enabled() {
            s.error
        } else {
            disabled_content(s.on_surface)
        };
        let pos = pos2(rect.left() + shape::SPACE_4, rect.top() + shape::SPACE_1);
        painter(ui).galley(pos, galley, color);
    }
}

/// (border stroke, label color) for the field's state.
fn colors(s: &Scheme, look: Look, error: bool) -> (Stroke, Color32) {
    let width = if look.focused { FOCUSED_BORDER } else { 1.0 };
    if !look.enabled {
        (
            Stroke::new(1.0, disabled_container(s.on_surface)),
            disabled_content(s.on_surface),
        )
    } else if error {
        (Stroke::new(width, s.error), s.error)
    } else if look.focused {
        (Stroke::new(FOCUSED_BORDER, s.primary), s.primary)
    } else if look.layer.is_some() {
        (Stroke::new(1.0, s.on_surface), s.on_surface_variant)
    } else {
        (Stroke::new(1.0, s.outline), s.on_surface_variant)
    }
}

/// The border with a gap cut for the label, then the label in the gap. The gap is cut by
/// drawing the border three times, clipped to the regions left of, right of and below the
/// label, so no background color has to be guessed to mask it.
fn paint(painter: &Painter, rect: Rect, stroke: Stroke, label_color: Color32, label: Arc<Galley>) {
    let size = label.size();
    let gap = Rect::from_min_size(
        pos2(rect.left() + shape::SPACE_3, rect.top() - size.y / 2.0),
        vec2(size.x + 2.0 * LABEL_GAP, size.y),
    );
    let outer = rect.expand(FOCUSED_BORDER * 2.0);
    let regions = [
        Rect::from_min_max(outer.min, pos2(gap.left(), outer.bottom())),
        Rect::from_min_max(pos2(gap.right(), outer.top()), outer.max),
        Rect::from_min_max(
            pos2(gap.left(), gap.bottom()),
            pos2(gap.right(), outer.bottom()),
        ),
    ];
    let corners = radius(shape::SMALL);
    for region in regions {
        painter
            .with_clip_rect(region)
            .rect_stroke(rect, corners, stroke, StrokeKind::Inside);
    }
    painter.galley(pos2(gap.left() + LABEL_GAP, gap.top()), label, label_color);
}
