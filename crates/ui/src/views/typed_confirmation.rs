//! The box that asks the person to type an exact text before a destructive step. The
//! instruction and the required text are wrapping, selectable labels above the box (an
//! outlined field's floating label is cut at the end, which would hide the very text that has
//! to be typed); the box keeps a short floating label and the full prompt as its accessible
//! name, so a screen reader still hears what to type.
use crate::theme::{FieldOptions, outlined_field_opts};
use eframe::egui::{self, Response, RichText};

/// What to show above and on the confirmation box.
pub struct TypedConfirmation<'a> {
    /// "Type this text exactly:" (no text in it).
    pub instruction: &'a str,
    /// The text that has to be typed.
    pub required: &'a str,
    /// The short label floating on the box.
    pub field_label: &'a str,
    /// The box's accessible name: the instruction with the required text.
    pub accessible_name: &'a str,
}

/// Draws the instruction, the required text on its own line (monospace, selectable so it can be
/// copied) and the box editing `text`.
pub fn show_typed_confirmation(
    ui: &mut egui::Ui,
    prompt: &TypedConfirmation<'_>,
    text: &mut String,
) -> Response {
    ui.label(prompt.instruction);
    ui.add(egui::Label::new(RichText::new(prompt.required).strong().monospace()).selectable(true));
    let options = FieldOptions {
        accessible_name: Some(prompt.accessible_name),
        ..FieldOptions::default()
    };
    outlined_field_opts(ui, prompt.field_label, text, "", options)
}
