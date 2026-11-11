//! The Shell tab's History panel: a toggle that shows or hides it (with "Clear history" beside it
//! while the open panel has runs), then (when open) the loading, unavailable or empty line, or
//! the remembered runs of the connection, newest first. Each run is ONE keyboard-activatable
//! button named by its status, when it ran and its first line; clicking it puts the script back
//! in the editor. Below the list: what is stored, and the inline confirm step of a clear. The
//! list is at most as tall as the tab's layout allows (see `shell_layout`). It only draws and
//! reports; every decision is the reducer's, and an action comes from a click (or a key press on
//! a focused button), never from a frame.
use super::tone_notice::tone_notice;
use crate::I18n;
use crate::app::shell_actions::ShellAction;
use crate::theme::shape::{self, radius};
use crate::theme::widgets::{
    CHIP_HEIGHT, Look, Ring, finite_width, focus_ring, medium_font, one_row, painter,
};
use crate::theme::{Tone, current_ui, icons, text_button, text_button_with_icon, tonal_button};
use crate::viewmodel::index_detail::Age;
use crate::viewmodel::shell_history::{HistoryView, ShellHistoryEntry, first_line};
use curlybrace_core::history::ShellRunStatus;
use eframe::egui::{self, Galley, Pos2, Rect, Sense, TextStyle, Vec2, WidgetInfo, WidgetType};
use std::sync::Arc;
use uuid::Uuid;

/// The salt of every id in a session's panel (its buttons and its scroll position), so two
/// sessions never share one.
const SALT: &str = "shell_history";
/// The prefix of every row's id; [`row_id`] adds the session and the entry.
const ROW_ID: &str = "shell_history_row";
/// The script's first line is cut to this many characters (the row elides it further to fit).
const FIRST_LINE_CHARS: usize = 80;
/// A row's height (points): one line of text with the status badge.
const ROW_HEIGHT: f32 = 32.0;
/// The column the time takes in a row, so the badges line up.
const TIME_WIDTH: f32 = 88.0;
/// The widest a status badge's text may be.
const BADGE_TEXT_WIDTH: f32 = 140.0;
/// The row width used when the room is unbounded (a horizontal scroll area).
const FALLBACK_WIDTH: f32 = 600.0;

/// The id of `session`'s row for the entry `entry_id`: stable across frames and new entries.
fn row_id(session: Uuid, entry_id: i64) -> egui::Id {
    egui::Id::new((ROW_ID, session, entry_id))
}

/// What the panel did in one frame.
pub(crate) struct HistoryShown {
    /// What was clicked, if anything.
    pub action: Option<ShellAction>,
    /// The height the list of runs took (points); 0 without a list.
    pub list_height: f32,
}

/// Draws the panel of `session`, its list at most `list_max` points tall (it scrolls past that).
pub(crate) fn show_history(
    ui: &mut egui::Ui,
    history: &HistoryView,
    session: Uuid,
    now_secs: i64,
    list_max: f32,
    i18n: &I18n,
) -> HistoryShown {
    ui.push_id((SALT, session), |ui| {
        // "Clear history" shares the toggle's row, so the open panel costs one row less.
        let toggled = ui
            .horizontal(|ui| {
                let toggled = toggle(ui, history.open, i18n);
                let clear = match offers_clear(history) {
                    true => text_button(ui, i18n.t("shell.history.clear"))
                        .clicked()
                        .then_some(ShellAction::ClearHistory),
                    false => None,
                };
                toggled.or(clear)
            })
            .inner;
        let place = Place {
            session,
            now_secs,
            list_max,
        };
        let inside = match history.open {
            true => body(ui, history, place, i18n),
            false => HistoryShown {
                action: None,
                list_height: 0.0,
            },
        };
        HistoryShown {
            action: toggled.or(inside.action),
            ..inside
        }
    })
    .inner
}

/// Where the panel is drawn: its session, the frame's time and the room for the list.
#[derive(Clone, Copy)]
struct Place {
    session: Uuid,
    now_secs: i64,
    list_max: f32,
}

/// The header button. It shows the title; its accessible name and tooltip say what pressing it
/// does, and assistive technology also gets its expanded state.
fn toggle(ui: &mut egui::Ui, open: bool, i18n: &I18n) -> Option<ShellAction> {
    let (icon, key) = match open {
        true => (icons::CHEVRON_DOWN, "shell.history.toggle_close"),
        false => (icons::CHEVRON_RIGHT, "shell.history.toggle_open"),
    };
    let name = i18n.t(key);
    let response = text_button_with_icon(ui, icon, &i18n.t("shell.history.title"));
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &name));
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_expanded(open));
    let clicked = response.on_hover_text(&name).clicked();
    clicked.then_some(ShellAction::ToggleHistory)
}

/// The open panel: loading, unavailable, or the list with its note and clear controls.
fn body(ui: &mut egui::Ui, history: &HistoryView, place: Place, i18n: &I18n) -> HistoryShown {
    let line = |ui: &mut egui::Ui, key: &str| {
        ui.label(i18n.t(key));
        HistoryShown {
            action: None,
            list_height: 0.0,
        }
    };
    match (history.loaded, history.available) {
        (false, _) => line(ui, "shell.history.loading"),
        (true, false) => line(ui, "shell.history.unavailable"),
        (true, true) => list_section(ui, history, place, i18n),
    }
}

fn list_section(
    ui: &mut egui::Ui,
    history: &HistoryView,
    place: Place,
    i18n: &I18n,
) -> HistoryShown {
    let entries = &history.entries;
    let (recalled, list_height) = match entries.is_empty() {
        true => {
            ui.label(i18n.t("shell.history.empty"));
            (None, 0.0)
        }
        false => rows(ui, entries, place, i18n),
    };
    ui.label(egui::RichText::new(i18n.t("shell.history.plaintext_note")).small());
    let cleared = match !entries.is_empty() && history.confirm_clear {
        true => confirm_controls(ui, i18n),
        false => None,
    };
    HistoryShown {
        action: recalled.or(cleared),
        list_height,
    }
}

/// The virtualised list: only the rows in view are laid out, so 200 entries cost a few rows.
/// Returns the recalled entry, if any, and the height the list took.
fn rows(
    ui: &mut egui::Ui,
    entries: &[ShellHistoryEntry],
    place: Place,
    i18n: &I18n,
) -> (Option<ShellAction>, f32) {
    let top = ui.cursor().min.y;
    let recalled = egui::ScrollArea::vertical()
        .id_salt((SALT, place.session))
        .max_height(place.list_max)
        // egui's own floor (64 points) would override a smaller room.
        .min_scrolled_height(place.list_max)
        .auto_shrink([false, true])
        .show_rows(ui, ROW_HEIGHT, entries.len(), |ui, range| {
            entries
                .get(range)
                .unwrap_or_default()
                .iter()
                .map(|entry| row(ui, entry, place.session, place.now_secs, i18n))
                .fold(None, Option::or)
        })
        .inner;
    (recalled, ui.cursor().min.y - top)
}

/// One run as one button: time, status badge and first line, read out in one name.
fn row(
    ui: &mut egui::Ui,
    entry: &ShellHistoryEntry,
    session: Uuid,
    now_secs: i64,
    i18n: &I18n,
) -> Option<ShellAction> {
    let texts = RowTexts {
        time: ago(entry.ran_at_secs, now_secs, i18n),
        status: i18n.t(status_key(entry.status)),
        line: first_line(&entry.script, FIRST_LINE_CHARS),
    };
    let name = i18n.t_with(
        "shell.history.row_label",
        &[
            ("status", &texts.status),
            ("time", &texts.time),
            ("script", &texts.line),
        ],
    );
    let width = finite_width(ui, FALLBACK_WIDTH);
    let (_, rect) = ui.allocate_space(Vec2::new(width, ROW_HEIGHT));
    let response = ui.interact(rect, row_id(session, entry.id), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &name));
    if ui.is_rect_visible(rect) {
        paint_row(ui, &response, status_tone(entry.status), &texts);
    }
    let clicked = response
        .on_hover_text(i18n.t("shell.history.recall_hint"))
        .clicked();
    clicked.then_some(ShellAction::RecallEntry(entry.id))
}

/// The translated parts of a row.
struct RowTexts {
    time: String,
    status: String,
    line: String,
}

fn paint_row(ui: &egui::Ui, response: &egui::Response, tone: Tone, texts: &RowTexts) {
    let s = current_ui(ui);
    let look = Look::of(ui, response);
    let rect = response.rect;
    let corners = radius(shape::SMALL);
    let painter = painter(ui);
    painter.rect_filled(rect, corners, look.overlay(s.on_surface));
    let small = TextStyle::Small.resolve(ui.style());
    let left = rect.min.x + shape::SPACE_2;
    let time = one_row(ui, &texts.time, small.clone(), TIME_WIDTH);
    painter.galley(centred(left, rect, &time), time, s.on_surface_variant);
    let badge_left = left + TIME_WIDTH + shape::SPACE_2;
    let badge_right = paint_badge(ui, &painter, rect, badge_left, tone, &texts.status);
    let line_left = badge_right + shape::SPACE_3;
    let mono = TextStyle::Monospace.resolve(ui.style());
    let room = rect.max.x - shape::SPACE_2 - line_left;
    let line = one_row(ui, &texts.line, mono, room);
    painter.galley(centred(line_left, rect, &line), line, s.on_surface);
    focus_ring(&painter, rect, corners, look, s.primary, Ring::Inside);
}

/// The status in words on the tone's container (the color only backs the words up); returns
/// the badge's right edge.
fn paint_badge(
    ui: &egui::Ui,
    painter: &egui::Painter,
    row: Rect,
    left: f32,
    tone: Tone,
    text: &str,
) -> f32 {
    let colors = current_ui(ui).tone(tone);
    let size = TextStyle::Small.resolve(ui.style()).size;
    let galley = one_row(ui, text, medium_font(ui, size), BADGE_TEXT_WIDTH);
    let badge = Rect::from_min_size(
        Pos2::new(left, row.center().y - CHIP_HEIGHT / 2.0),
        Vec2::new(galley.size().x + 2.0 * shape::SPACE_3, CHIP_HEIGHT),
    );
    painter.rect_filled(badge, radius(shape::FULL), colors.container);
    painter.galley(
        badge.center() - galley.size() / 2.0,
        galley,
        colors.on_container,
    );
    badge.max.x
}

/// Where `galley` starts so it sits at `left`, centred on the row's height.
fn centred(left: f32, row: Rect, galley: &Arc<Galley>) -> Pos2 {
    Pos2::new(left, row.center().y - galley.size().y / 2.0)
}

/// Whether the header offers "Clear history": the panel is open on a non-empty list and no clear
/// is waiting for its confirmation.
fn offers_clear(history: &HistoryView) -> bool {
    history.open
        && history.loaded
        && history.available
        && !history.entries.is_empty()
        && !history.confirm_clear
}

/// Once "Clear history" was pressed: the question with its Yes and Keep buttons.
fn confirm_controls(ui: &mut egui::Ui, i18n: &I18n) -> Option<ShellAction> {
    tone_notice(
        ui,
        Tone::Warning,
        &i18n.t("shell.history.clear_confirm"),
        false,
    );
    ui.horizontal_wrapped(|ui| {
        let yes = tonal_button(ui, i18n.t("shell.history.clear_confirm_yes"));
        let keep = text_button(ui, i18n.t("shell.history.clear_cancel"));
        match (yes.clicked(), keep.clicked()) {
            (true, _) => Some(ShellAction::ConfirmClear),
            (_, true) => Some(ShellAction::CancelClear),
            _ => None,
        }
    })
    .inner
}

/// How long before `now_secs` the run was, in the words the Indexes tab uses (a clock that went
/// backwards reads "just now", never a time in the future).
fn ago(ran_at_secs: i64, now_secs: i64, i18n: &I18n) -> String {
    let age = Age::of(now_secs.saturating_sub(ran_at_secs).max(0));
    i18n.t_with(age.key(), &[("n", &age.count().to_string())])
}

fn status_key(status: ShellRunStatus) -> &'static str {
    match status {
        ShellRunStatus::Succeeded => "shell.history.status.succeeded",
        ShellRunStatus::ScriptErrors => "shell.history.status.script_errors",
        ShellRunStatus::Incomplete => "shell.history.status.incomplete",
        ShellRunStatus::Cancelled => "shell.history.status.cancelled",
        ShellRunStatus::CapReached => "shell.history.status.cap_reached",
        ShellRunStatus::Failed => "shell.history.status.failed",
    }
}

fn status_tone(status: ShellRunStatus) -> Tone {
    match status {
        ShellRunStatus::Succeeded => Tone::Good,
        ShellRunStatus::Cancelled => Tone::Neutral,
        ShellRunStatus::Incomplete | ShellRunStatus::CapReached => Tone::Warning,
        ShellRunStatus::ScriptErrors | ShellRunStatus::Failed => Tone::Bad,
    }
}

#[cfg(test)]
#[path = "shell_history_tests.rs"]
mod tests;
