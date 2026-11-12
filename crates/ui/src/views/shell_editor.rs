//! The Shell tab's script editor: a highlighted multi-line text box that reports what the user
//! typed or selected. The view-model keeps the selection as BYTE offsets; egui's cursor counts
//! characters, so both directions are converted here.
//!
//! Each session has its own editor (its id is salted with the session), so the cursor, the
//! selection, the undo history and the scroll offset of one session never leak into another.
//!
//! The selection held in the state is pushed into the text box only when the state changed the
//! script or the selection on its own (Format, Go to problem, the first frame of this session's
//! editor), so the caret is not reset on every frame. "Changed on its own" means: the script's
//! hash, or the selection, differs from the one this editor last saw or sent. A state without a
//! selection then collapses the text box's selection, so an old one is never reported as the new
//! script's. The editor takes the focus only when the state asks for it (`editor_focus`, bumped
//! by Go to problem), once per request, since egui shows a selection only in a focused text box;
//! other state-only changes (Format, history recall) leave the focus where it is.
//!
//! The completion list (`shell_complete`) reads its keys before the text box is drawn, so Enter,
//! Tab, Up, Down and Esc reach the text box as before whenever the list is closed.
use super::shell_complete::{
    Moment, Popup, applied, clicking_popup, next_popup, put_pick, read_keys, show_popup,
    typed_trigger,
};
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

/// Keys of what an editor remembers between frames (salted with its id).
const SEEN: &str = "seen";
const SEEN_SELECTION: &str = "seen_selection";
const SEEN_FOCUS: &str = "seen_focus";
const POPUP: &str = "complete";

/// The fewest rows the editor shows.
const MIN_ROWS: usize = 4;

/// The id of the editor of `session`'s Shell tab.
pub(crate) fn editor_id(session: Uuid) -> egui::Id {
    egui::Id::new(EDITOR_ID).with(session)
}

/// Draws the editor of `session`, `height` points tall, with its completion list (fed with
/// `collections`); an `Edit` when the text or the selection changed, or a suggestion was picked.
pub(crate) fn show_editor(
    ui: &mut egui::Ui,
    state: &ShellState,
    session: Uuid,
    height: f32,
    collections: &[String],
    i18n: &I18n,
) -> Option<ShellAction> {
    let id = editor_id(session);
    let (script_hash, outside_change) = sync_from_state(ui, id, state);
    // A list computed for text the state has since replaced would pick into the wrong bytes.
    let popup = ui
        .data(|d| d.get_temp::<Popup>(id.with(POPUP)))
        .filter(|_| !outside_change);
    let keys = read_keys(ui, id, popup.as_ref());
    let popup = popup.map(|p| Popup {
        selected: keys.selected.unwrap_or(p.selected),
        ..p
    });
    let typed = ui.memory(|m| m.has_focus(id)) && typed_trigger(ui);
    let key_pick = popup
        .as_ref()
        .filter(|_| keys.pick)
        .map(|p| picked(ui.ctx(), id, &state.script, p, p.selected));
    let popup = popup.filter(|_| !keys.pick && !keys.close);
    let mut buffer = key_pick.clone().unwrap_or_else(|| state.script.clone());
    let output = text_box(ui, id, &mut buffer, height, i18n);
    let caret = caret_of(&output, &buffer);
    let moment = Moment {
        text: &buffer,
        caret: caret.map(|(byte, _)| byte),
        focused: ui.memory(|m| m.has_focus(id)) || clicking_popup(ui.ctx(), id),
        typed,
        forced: keys.forced,
        changed: output.response.changed(),
    };
    let caret_rect = caret.map(|(_, rect)| rect);
    let click_pick = follow_list(ui, id, popup, &moment, caret_rect, collections, i18n);
    let was_picked = key_pick.is_some() || click_pick.is_some();
    let buffer = click_pick.unwrap_or(buffer);
    let selection = match was_picked {
        true => None,
        false => output
            .state
            .cursor
            .char_range()
            .and_then(|range| char_range_to_bytes(&buffer, range.as_sorted_char_range())),
    };
    // The state's collapsed caret (Go to problem) is no selection: the text box reports it as
    // `None`, which is not an edit.
    let state_selection = state.selection.clone().filter(|r| r.start < r.end);
    let edited = was_picked || output.response.changed() || selection != state_selection;
    remember_seen(ui, id, edited, script_hash, &buffer, &selection, state);
    edited.then_some(ShellAction::Edit {
        script: buffer,
        selection,
    })
}

/// The completion list after `moment`: opened, followed or closed, drawn against `caret` and
/// remembered for the next frame. Returns the text with a clicked item picked.
fn follow_list(
    ui: &mut egui::Ui,
    id: egui::Id,
    popup: Option<Popup>,
    moment: &Moment<'_>,
    caret: Option<egui::Rect>,
    collections: &[String],
    i18n: &I18n,
) -> Option<String> {
    let popup = next_popup(popup, moment, collections);
    let click_pick = popup.as_ref().zip(caret).and_then(|(p, rect)| {
        let index = show_popup(ui, id, rect, p, i18n)?;
        ui.memory_mut(|m| m.request_focus(id));
        Some(picked(ui.ctx(), id, moment.text, p, index))
    });
    let open = popup.filter(|_| click_pick.is_none());
    ui.data_mut(|d| match open {
        Some(open) => d.insert_temp(id.with(POPUP), open),
        None => d.remove::<Popup>(id.with(POPUP)),
    });
    click_pick
}

/// Pushes the state's selection into the text box when the state changed the script or the
/// selection on its own, and focuses the editor once for every focus request it has not seen
/// (Go to problem). The first frame of an editor takes no focus. Returns the script's hash and
/// whether the state changed anything on its own.
fn sync_from_state(ui: &mut egui::Ui, id: egui::Id, state: &ShellState) -> (u64, bool) {
    let script_hash = egui::util::hash(&state.script);
    let script_changed = ui.data(|d| d.get_temp::<u64>(id.with(SEEN))) != Some(script_hash);
    let seen_selection = ui.data(|d| d.get_temp::<Option<Range<usize>>>(id.with(SEEN_SELECTION)));
    let selection_moved = !script_changed && seen_selection.as_ref() != Some(&state.selection);
    let seen_focus = ui.data(|d| d.get_temp::<u64>(id.with(SEEN_FOCUS)));
    // Only a newer request: a shell state created anew for the session starts over at 0.
    let focus_asked = seen_focus.is_some_and(|seen| state.editor_focus > seen);
    if script_changed || selection_moved || focus_asked {
        apply_selection(ui.ctx(), id, &state.script, state.selection.as_ref());
    }
    if focus_asked {
        // Go to problem: the selection only shows in a focused text box.
        ui.memory_mut(|m| m.request_focus(id));
    }
    ui.data_mut(|d| d.insert_temp(id.with(SEEN_FOCUS), state.editor_focus));
    (
        script_hash,
        script_changed || selection_moved || focus_asked,
    )
}

/// Remembers the script and selection this editor last saw or sent.
fn remember_seen(
    ui: &mut egui::Ui,
    id: egui::Id,
    edited: bool,
    script_hash: u64,
    buffer: &str,
    selection: &Option<Range<usize>>,
    state: &ShellState,
) {
    let (seen, seen_selection) = match edited {
        true => (egui::util::hash(buffer), selection.clone()),
        false => (script_hash, state.selection.clone()),
    };
    ui.data_mut(|d| {
        d.insert_temp(id.with(SEEN), seen);
        d.insert_temp(id.with(SEEN_SELECTION), seen_selection);
    });
}

/// The highlighted text box itself, in its scroll area.
fn text_box(
    ui: &mut egui::Ui,
    id: egui::Id,
    buffer: &mut String,
    height: f32,
    i18n: &I18n,
) -> egui::text_edit::TextEditOutput {
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
            egui::TextEdit::multiline(buffer)
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
    output
}

/// The caret as a byte of `text` and its rect on screen; `None` with a selection.
fn caret_of(output: &egui::text_edit::TextEditOutput, text: &str) -> Option<(usize, egui::Rect)> {
    let range = output.state.cursor.char_range()?;
    if range.primary.index != range.secondary.index {
        return None;
    }
    let rect = output.galley.pos_from_cursor(range.primary);
    let byte = byte_of_char(text, range.primary.index);
    Some((byte, rect.translate(output.galley_pos.to_vec2())))
}

/// `text` with item `index` of `popup` picked, the caret put after the insertion.
fn picked(ctx: &egui::Context, id: egui::Id, text: &str, popup: &Popup, index: usize) -> String {
    let item = &popup.completion.items[index];
    let (next, caret) = applied(text, &popup.completion.replace, &item.insert);
    put_pick(ctx, id, text, &next, caret);
    next
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

pub(crate) fn byte_of_char(text: &str, index: usize) -> usize {
    text.char_indices()
        .nth(index)
        .map_or(text.len(), |(byte, _)| byte)
}

#[cfg(test)]
#[path = "shell_editor_tests.rs"]
mod tests;
