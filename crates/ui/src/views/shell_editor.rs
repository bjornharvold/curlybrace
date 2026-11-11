//! The Shell tab's script editor: a highlighted multi-line text box that reports what the user
//! typed or selected. The view-model keeps the selection as BYTE offsets; egui's cursor counts
//! characters, so both directions are converted here.
//!
//! Each session has its own editor (its id is salted with the session), so the cursor, the
//! selection, the undo history and the scroll offset of one session never leak into another.
//!
//! The selection held in the state is pushed into the text box only when the state changed the
//! script on its own (Format, the first frame of this session's editor), so the caret is not
//! reset on every frame. "Changed on its own" means: the script's hash differs from the one this
//! editor last saw or sent. A state without a selection then collapses the text box's selection,
//! so an old one is never reported as the new script's.
use super::shell_highlight::cached_highlight;
use crate::I18n;
use crate::app::shell_actions::ShellAction;
use crate::theme::{current_ui, js_colors};
use crate::viewmodel::shell::ShellState;
use eframe::egui::{self, TextBuffer, text::CCursorRange};
use std::ops::Range;
use uuid::Uuid;

/// The prefix of every editor's id; [`editor_id`] salts it with the session.
const EDITOR_ID: &str = "shell_editor";

/// The fewest rows the editor shows.
const MIN_ROWS: usize = 4;

/// The id of the editor of `session`'s Shell tab.
pub(crate) fn editor_id(session: Uuid) -> egui::Id {
    egui::Id::new(EDITOR_ID).with(session)
}

/// Draws the editor of `session`, `height` points tall; an `Edit` when the text or the
/// selection changed.
pub(crate) fn show_editor(
    ui: &mut egui::Ui,
    state: &ShellState,
    session: Uuid,
    height: f32,
    i18n: &I18n,
) -> Option<ShellAction> {
    let id = editor_id(session);
    let seen_id = id.with("seen");
    let script_hash = egui::util::hash(&state.script);
    if ui.data(|d| d.get_temp::<u64>(seen_id)) != Some(script_hash) {
        apply_selection(ui.ctx(), id, &state.script, state.selection.as_ref());
    }
    let mut buffer = state.script.clone();
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let colors = js_colors(current_ui(ui));
    let cache = id.with("highlight");
    let mut layouter = |ui: &egui::Ui, text: &dyn TextBuffer, wrap_width: f32| {
        let job = cached_highlight(ui.ctx(), cache, text.as_str(), &font, &colors, wrap_width);
        ui.fonts(|f| f.layout_job((*job).clone()))
    };
    let rows =
        ((height / ui.text_style_height(&egui::TextStyle::Monospace)) as usize).max(MIN_ROWS);
    let placeholder = i18n.t("shell.editor_placeholder");
    let output = egui::ScrollArea::vertical()
        .id_salt(id.with("scroll"))
        .max_height(height)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            egui::TextEdit::multiline(&mut buffer)
                .id(id)
                .code_editor()
                .desired_rows(rows)
                .desired_width(f32::INFINITY)
                .hint_text(placeholder.clone())
                .layouter(&mut layouter)
                .show(ui)
        })
        .inner;
    let name = i18n.t("shell.editor_label");
    ui.ctx().accesskit_node_builder(output.response.id, |node| {
        node.set_label(name);
        node.set_placeholder(placeholder);
    });
    let selection = output
        .state
        .cursor
        .char_range()
        .and_then(|range| char_range_to_bytes(&buffer, range.as_sorted_char_range()));
    let edited = output.response.changed() || selection != state.selection;
    let seen = if edited {
        egui::util::hash(&buffer)
    } else {
        script_hash
    };
    ui.data_mut(|d| d.insert_temp(seen_id, seen));
    edited.then_some(ShellAction::Edit {
        script: buffer,
        selection,
    })
}

/// Puts `selection` (bytes of `script`) into the text box's cursor. Without one, the selection
/// collapses onto its active end (egui keeps that end inside the text).
fn apply_selection(
    ctx: &egui::Context,
    id: egui::Id,
    script: &str,
    selection: Option<&Range<usize>>,
) {
    let mut text_state = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
    let range = match selection {
        Some(bytes) => {
            let chars = byte_range_to_chars(script, bytes);
            CCursorRange::two(
                egui::text::CCursor::new(chars.start),
                egui::text::CCursor::new(chars.end),
            )
        }
        None => {
            let Some(current) = text_state.cursor.char_range() else {
                return;
            };
            let end = current.primary.index.min(script.chars().count());
            CCursorRange::one(egui::text::CCursor::new(end))
        }
    };
    text_state.cursor.set_char_range(Some(range));
    text_state.store(ctx, id);
}

/// The byte range of the characters `chars` of `text`; `None` when it selects nothing.
pub(crate) fn char_range_to_bytes(text: &str, chars: Range<usize>) -> Option<Range<usize>> {
    if chars.start >= chars.end {
        return None;
    }
    let start = byte_of_char(text, chars.start);
    let end = byte_of_char(text, chars.end);
    (start < end).then_some(start..end)
}

/// The character range of the bytes `bytes` of `text`, each end moved back to a character
/// boundary and kept inside the text.
pub(crate) fn byte_range_to_chars(text: &str, bytes: &Range<usize>) -> Range<usize> {
    let char_of = |byte: usize| {
        let mut at = byte.min(text.len());
        while !text.is_char_boundary(at) {
            at -= 1;
        }
        text[..at].chars().count()
    };
    char_of(bytes.start)..char_of(bytes.end)
}

fn byte_of_char(text: &str, index: usize) -> usize {
    text.char_indices()
        .nth(index)
        .map_or(text.len(), |(byte, _)| byte)
}

#[cfg(test)]
#[path = "shell_editor_tests.rs"]
mod tests;
