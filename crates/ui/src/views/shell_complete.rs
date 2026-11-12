//! The Shell editor's completion list: when it opens, follows the typed word and closes, which
//! keys it takes, how a pick lands in the text, and the list itself (under the caret, or above it
//! when there is no room below).
//!
//! Rules: the list opens on typing a word character, `.` or `$` (never on whitespace, Enter or an
//! arrow) and on `Ctrl+Space`. It closes on Esc, a pick, the caret leaving the word, a selection,
//! the editor losing the focus, or no names left. Only while it is open (and then an item is
//! always selected) does it take Up, Down, Enter, Tab and Esc; otherwise those keys reach the
//! text box as before. The open list lives in egui temp data next to the editor's own state.
use super::shell_editor::byte_range_to_chars;
use crate::I18n;
use crate::viewmodel::shell_complete::{
    Completion, Suggestion, SuggestionKind, complete, complete_forced, is_word_byte, word_start,
};
use eframe::egui::{self, Key, Modifiers, accesskit};
use std::ops::Range;

/// Where the editor remembers the caret's rect the list was placed against (screen points).
pub(crate) const CARET_RECT: &str = "complete_caret";

/// The open list.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Popup {
    pub completion: Completion,
    /// Index of the selected item; always inside `completion.items`.
    pub selected: usize,
    /// Byte where the completed word starts; the caret leaving that word closes the list.
    pub word_start: usize,
    /// The caret (bytes) the names were computed for.
    pub caret: usize,
    /// Opened on demand, so an emptied word still lists names.
    pub forced: bool,
}

/// What happened in the editor this frame, as far as the list is concerned.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Moment<'a> {
    pub text: &'a str,
    /// The caret in bytes; `None` with a selection.
    pub caret: Option<usize>,
    pub focused: bool,
    /// A word character, `.` or `$` was typed.
    pub typed: bool,
    /// `Ctrl+Space` was pressed.
    pub forced: bool,
    /// The text changed.
    pub changed: bool,
}

/// The list after `moment`: opened, followed, kept or closed.
pub(crate) fn next_popup(
    prev: Option<Popup>,
    moment: &Moment<'_>,
    collections: &[String],
) -> Option<Popup> {
    let caret = moment.caret.filter(|_| moment.focused)?;
    let start = word_start(moment.text.as_bytes(), caret.min(moment.text.len()));
    if moment.forced {
        return opened(moment.text, caret, start, true, collections);
    }
    match prev {
        Some(prev) if prev.word_start == start => {
            if !moment.changed && prev.caret == caret {
                return Some(prev);
            }
            let next = opened(moment.text, caret, start, prev.forced, collections)?;
            let label = &prev.completion.items[prev.selected].label;
            let selected = next
                .completion
                .items
                .iter()
                .position(|s| &s.label == label)
                .unwrap_or(0);
            Some(Popup { selected, ..next })
        }
        _ if moment.typed => opened(moment.text, caret, start, false, collections),
        _ => None,
    }
}

fn opened(
    text: &str,
    caret: usize,
    word_start: usize,
    forced: bool,
    collections: &[String],
) -> Option<Popup> {
    let completion = match forced {
        true => complete_forced(text, caret, collections),
        false => complete(text, caret, collections),
    }?;
    Some(Popup {
        completion,
        selected: 0,
        word_start,
        caret,
        forced,
    })
}

/// `text` with `replace` (and the rest of the word after it, so a pick in the middle of a word
/// does not double its tail) replaced by `insert`, and the caret (bytes) just after the insertion.
pub(crate) fn applied(text: &str, replace: &Range<usize>, insert: &str) -> (String, usize) {
    let bytes = text.as_bytes();
    let boundary = |at: usize| {
        (0..=at.min(text.len()))
            .rev()
            .find(|&i| text.is_char_boundary(i))
            .unwrap_or(0)
    };
    let start = boundary(replace.start);
    let end = (boundary(replace.end).max(start)..text.len())
        .find(|&i| !is_word_byte(bytes[i]))
        .unwrap_or(text.len());
    let next = format!("{}{insert}{}", &text[..start], &text[end..]);
    (next, start + insert.len())
}

/// Whether this frame typed a character that opens the list.
pub(crate) fn typed_trigger(ui: &egui::Ui) -> bool {
    ui.input(|i| {
        i.events.iter().any(|event| match event {
            egui::Event::Text(text) => text
                .chars()
                .last()
                .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '$' | '.')),
            _ => false,
        })
    })
}

/// The keys of this frame that the list takes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ListKeys {
    /// `Ctrl+Space`.
    pub forced: bool,
    /// The item selected after Up / Down (wrapping); `None` without an open list.
    pub selected: Option<usize>,
    /// Enter or Tab: pick the selected item.
    pub pick: bool,
    /// Esc: close the list.
    pub close: bool,
}

/// Reads (and consumes, before the text box sees them) the keys of the list. `Ctrl+Space` is
/// read whenever the editor has the focus; the rest only while `popup` is open. Esc keeps the
/// editor focused (egui drops the focus on Esc before any widget runs, so it is handed back).
pub(crate) fn read_keys(ui: &mut egui::Ui, editor: egui::Id, popup: Option<&Popup>) -> ListKeys {
    // Esc drops the focus to nobody at the start of the frame; any other focus move is not ours.
    let (focused, dropped_by_esc) = ui.memory(|m| {
        let dropped = m.had_focus_last_frame(editor) && m.focused().is_none();
        (m.has_focus(editor), dropped)
    });
    let forced = focused && ui.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::Space));
    let Some(popup) = popup.filter(|_| focused || dropped_by_esc) else {
        return ListKeys {
            forced,
            ..ListKeys::default()
        };
    };
    let (down, up, pick, close) = ui.input_mut(|i| {
        let down = i.count_and_consume_key(Modifiers::NONE, Key::ArrowDown);
        let up = i.count_and_consume_key(Modifiers::NONE, Key::ArrowUp);
        let enter = i.consume_key(Modifiers::NONE, Key::Enter);
        let tab = i.consume_key(Modifiers::NONE, Key::Tab);
        let close = i.consume_key(Modifiers::NONE, Key::Escape);
        (down, up, enter || tab, close)
    });
    if close {
        ui.memory_mut(|m| m.request_focus(editor));
    }
    ListKeys {
        forced,
        selected: Some(moved(
            popup.selected,
            down,
            up,
            popup.completion.items.len(),
        )),
        pick,
        close,
    }
}

/// `selected` moved `down` and `up` rows in a list of `count`, wrapping at both ends.
fn moved(selected: usize, down: usize, up: usize, count: usize) -> usize {
    let count = count.max(1) as isize;
    (selected as isize + down as isize - up as isize).rem_euclid(count) as usize
}

/// Puts the caret of the text box `editor` at byte `caret` of `text`.
pub(crate) fn put_caret(ctx: &egui::Context, editor: egui::Id, text: &str, caret: usize) {
    let mut state = egui::text_edit::TextEditState::load(ctx, editor).unwrap_or_default();
    let at = byte_range_to_chars(text, &(caret..caret)).start;
    let range = egui::text::CCursorRange::one(egui::text::CCursor::new(at));
    state.cursor.set_char_range(Some(range));
    state.store(ctx, editor);
}

/// The id of the list's area for the editor `editor`.
pub(crate) fn popup_area_id(editor: egui::Id) -> egui::Id {
    editor.with("complete_popup")
}

/// Whether a mouse button is pressed or released over the list drawn last frame: the press takes
/// the focus from the editor, and that must not close the list before the click lands.
pub(crate) fn clicking_popup(ctx: &egui::Context, editor: egui::Id) -> bool {
    let area = ctx.memory(|m| m.area_rect(popup_area_id(editor)));
    let pointer = ctx.input(|i| {
        let clicking = i.pointer.any_down() || i.pointer.any_released();
        i.pointer.latest_pos().filter(|_| clicking)
    });
    matches!((area, pointer), (Some(area), Some(at)) if area.contains(at))
}

/// Draws the list against `caret` (the caret's rect, screen points): under it, or above it when
/// it fits only there or the window has more room above. It never leaves the window: a list
/// taller than the room on its side scrolls. Returns the index of a clicked item.
pub(crate) fn show_popup(
    ui: &egui::Ui,
    editor: egui::Id,
    caret: egui::Rect,
    popup: &Popup,
    i18n: &I18n,
) -> Option<usize> {
    let ctx = ui.ctx().clone();
    ctx.data_mut(|d| d.insert_temp(editor.with(CARET_RECT), caret));
    let area_id = popup_area_id(editor);
    let height = ctx
        .memory(|m| m.area_rect(area_id))
        .map_or_else(|| estimated_height(ui, popup), |rect| rect.height());
    let screen = ctx.screen_rect();
    let (below, above) = (screen.bottom() - caret.bottom(), caret.top() - screen.top());
    let (pivot, at, room) = match height <= below || below >= above {
        true => (egui::Align2::LEFT_TOP, caret.left_bottom(), below),
        false => (egui::Align2::LEFT_BOTTOM, caret.left_top(), above),
    };
    egui::Area::new(area_id)
        .order(egui::Order::Foreground)
        .pivot(pivot)
        .fixed_pos(at)
        .constrain(true)
        .show(&ctx, |ui| {
            egui::Frame::popup(ui.style())
                .show(ui, |ui| {
                    let chrome = ui.spacing().menu_margin.sum().y + 2.0;
                    egui::ScrollArea::vertical()
                        .max_height((room - chrome).max(0.0))
                        .show(ui, |ui| list(ui, area_id, popup, i18n))
                        .inner
                })
                .inner
        })
        .inner
}

/// A first guess of the list's height, before it was ever drawn.
fn estimated_height(ui: &egui::Ui, popup: &Popup) -> f32 {
    let spacing = ui.spacing();
    let row = spacing.interact_size.y + spacing.item_spacing.y;
    popup.completion.items.len() as f32 * row + spacing.menu_margin.sum().y
}

/// The items as an accessible list: each a list item named by its text, the selected one marked.
fn list(ui: &mut egui::Ui, area: egui::Id, popup: &Popup, i18n: &I18n) -> Option<usize> {
    let ctx = ui.ctx().clone();
    let list_id = area.with("list");
    let name = i18n.t("shell.complete.list_label");
    ctx.accesskit_node_builder(list_id, |node| {
        node.set_role(accesskit::Role::List);
        node.set_label(name);
    });
    ctx.with_accessibility_parent(list_id, || {
        let layout = egui::Layout::top_down_justified(egui::Align::LEFT);
        ui.with_layout(layout, |ui| {
            let rows = popup.completion.items.iter().enumerate();
            rows.fold(None, |clicked, (index, item)| {
                let row = item_row(ui, item, index == popup.selected, i18n);
                clicked.or(row.clicked().then_some(index))
            })
        })
        .inner
    })
}

/// One item: its text in monospace and a small marker of its kind; a list item to accesskit.
fn item_row(ui: &mut egui::Ui, item: &Suggestion, selected: bool, i18n: &I18n) -> egui::Response {
    let text = egui::RichText::new(&item.label).monospace();
    let kind = egui::RichText::new(i18n.t(kind_key(item.kind)))
        .small()
        .weak();
    let row = ui.add(egui::Button::selectable(selected, text).right_text(kind));
    if selected && !ui.clip_rect().contains_rect(row.rect) {
        row.scroll_to_me(None);
    }
    let label = item.label.clone();
    ui.ctx().accesskit_node_builder(row.id, |node| {
        node.set_role(accesskit::Role::ListItem);
        node.set_label(label);
        node.set_selected(selected);
    });
    row
}

/// The i18n key of the small marker that says what kind of name an item is.
pub(crate) fn kind_key(kind: SuggestionKind) -> &'static str {
    match kind {
        SuggestionKind::Collection => "shell.complete.kind_collection",
        SuggestionKind::DbMethod => "shell.complete.kind_db_method",
        SuggestionKind::CollectionMethod => "shell.complete.kind_collection_method",
        SuggestionKind::Operator => "shell.complete.kind_operator",
        SuggestionKind::Word => "shell.complete.kind_word",
    }
}

#[cfg(test)]
#[path = "shell_complete_tests.rs"]
mod tests;
