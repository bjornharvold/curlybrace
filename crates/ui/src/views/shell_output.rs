//! The Shell tab's output pane: a named region with the `Text | Table | Tree` switch on its
//! heading row, then the run's text as virtualised, selectable monospace lines (or its value as a
//! table or a tree, see `shell_result`).
//! Stderr is drawn in the error color AND marked in words, so color is never the only signal:
//! sighted readers see a painted marker in front of the line, and a screen reader hears the
//! line's name, which says it is error output (the painted marker is not a node, so the cue is
//! spoken once). While a run is going
//! the pane follows the newest line unless the user scrolled away from the bottom.
use super::metrics::{fixed_row_sized, row_height};
use super::shell_result::{show_mode_switch, show_result, show_result_bar};
use super::shell_result_cache::drop_stale;
use crate::I18n;
use crate::app::shell_actions::ShellAction;
use crate::theme::current_ui;
use crate::viewmodel::shell::{OutputLine, ShellState};
use crate::viewmodel::shell_result::OutputMode;
use curlybrace_core::shell::Stream;
use eframe::egui::{self, accesskit};
use std::sync::Arc;
use uuid::Uuid;

const REGION_ID: &str = "shell_output_region";
/// The prefix of every output pane's scroll id; [`scroll_salt`] adds the session.
const SCROLL_ID: &str = "shell_output";

/// The scroll id salt of `session`'s output, so each session keeps its own scroll position (like
/// the editor keeps its own state).
fn scroll_salt(session: Uuid) -> (&'static str, Uuid) {
    (SCROLL_ID, session)
}

/// The output heading with the mode switch, then in Text mode the dropped-lines note and the
/// lines (or what to expect when there are none yet) of `session`, else the Table or Tree; all
/// inside one region named for assistive technology. The compact switch and the mode's own bar
/// (the Table's notes, the Tree's Expand all / Collapse all and notes) share the heading's row,
/// so a short pane (800x500 with History open) still shows rows.
pub(crate) fn show_output(
    ui: &mut egui::Ui,
    state: &ShellState,
    session: Uuid,
    i18n: &I18n,
) -> Option<ShellAction> {
    let ctx = ui.ctx().clone();
    // Rows built for a value no longer shown (an earlier run, a cleared value) are dropped at
    // once, in both modes, whichever one is on screen.
    drop_stale(&ctx, session, state);
    let region = egui::Id::new(REGION_ID);
    let name = i18n.t("shell.output.region");
    ctx.accesskit_node_builder(region, |node| {
        node.set_role(accesskit::Role::Region);
        node.set_label(name);
    });
    let (rect, action) = ctx.with_accessibility_parent(region, || {
        let scope = ui.scope(|ui| {
            let switched = ui
                .horizontal_wrapped(|ui| {
                    ui.strong(i18n.t("shell.output.title"));
                    ui.add_space(ui.spacing().item_spacing.x * 2.0);
                    let switched = show_mode_switch(ui, state, i18n);
                    ui.add_space(ui.spacing().item_spacing.x * 2.0);
                    let bar = show_result_bar(ui, state, session, i18n);
                    switched.or(bar)
                })
                .inner;
            let shown = match state.mode {
                OutputMode::Text => {
                    show_text(ui, state, session, i18n);
                    None
                }
                OutputMode::Table | OutputMode::Tree => show_result(ui, state, session, i18n),
            };
            switched.or(shown)
        });
        (scope.response.rect, scope.inner)
    });
    ctx.accesskit_node_builder(region, |node| {
        node.set_bounds(accesskit::Rect {
            x0: rect.min.x.into(),
            y0: rect.min.y.into(),
            x1: rect.max.x.into(),
            y1: rect.max.y.into(),
        });
    });
    action
}

/// The Text mode: the dropped-lines note and the lines, or what to expect when there are none.
fn show_text(ui: &mut egui::Ui, state: &ShellState, session: Uuid, i18n: &I18n) {
    if state.dropped_lines > 0 {
        let count = state.dropped_lines.to_string();
        let key = match state.dropped_lines {
            1 => "shell.output.dropped_one",
            _ => "shell.output.dropped_other",
        };
        ui.label(i18n.t_with(key, &[("count", &count)]));
    }
    if state.line_count() == 0 {
        ui.label(i18n.t(empty_key(state)));
    } else {
        show_lines(ui, state, session, i18n);
    }
}

fn empty_key(state: &ShellState) -> &'static str {
    match state.is_running() {
        true => "shell.output.waiting",
        false => "shell.output.empty",
    }
}

fn show_lines(ui: &mut egui::Ui, state: &ShellState, session: Uuid, i18n: &I18n) {
    let height = row_height(ui);
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let normal = ui.visuals().text_color();
    let error = current_ui(ui).error;
    let marker = i18n.t("shell.output.stderr_marker");
    let truncated = i18n.t("shell.output.truncated");
    egui::ScrollArea::both()
        .id_salt(scroll_salt(session))
        .auto_shrink([false; 2])
        .stick_to_bottom(state.is_running())
        .show_rows(ui, height, state.line_count(), |ui, range| {
            for index in range {
                let Some(line) = state.line(index) else {
                    continue;
                };
                let shown = shown_text(line, &truncated);
                let is_error = line.stream == Stream::Stderr;
                let color = if is_error { error } else { normal };
                let name =
                    is_error.then(|| i18n.t_with("shell.output.stderr_line", &[("text", &shown)]));
                let galley = ui.fonts(|f| f.layout_no_wrap(shown, font.clone(), color));
                let mark = is_error.then(|| {
                    let mark = format!("{marker} ");
                    ui.fonts(|f| f.layout_no_wrap(mark, font.clone(), error))
                });
                let mark_width = mark.as_ref().map_or(0.0, |m| m.size().x);
                let width = (galley.size().x + mark_width).max(ui.available_width());
                fixed_row_sized(ui, height, width, |ui| {
                    if let Some(mark) = mark {
                        paint_marker(ui, mark, error);
                    }
                    line_label(ui, galley, name);
                });
            }
        });
}

/// Paints the stderr marker and leaves room for it, without a widget: the line's accessible
/// name already says "error", so the marker must not be announced a second time.
fn paint_marker(ui: &mut egui::Ui, mark: Arc<egui::Galley>, color: egui::Color32) {
    let room = ui.available_rect_before_wrap();
    let top = room.center().y - mark.size().y / 2.0;
    let width = mark.size().x;
    ui.painter()
        .galley(egui::pos2(room.min.x, top), mark, color);
    ui.add_space(width);
}

/// The line as drawn: an over-long line ends with the "cut" marker, one space after the text
/// (the space is not part of the translation, so it cannot be lost there).
fn shown_text(line: &OutputLine, truncated: &str) -> String {
    match line.truncated {
        true => format!("{} {truncated}", line.text),
        false => line.text.clone(),
    }
}

/// A selectable line; `name`, when given, replaces the drawn text as its accessible name.
fn line_label(ui: &mut egui::Ui, galley: Arc<egui::Galley>, name: Option<String>) {
    let label = egui::Label::new(egui::WidgetText::Galley(galley))
        .wrap_mode(egui::TextWrapMode::Extend)
        .selectable(true);
    let response = ui.add(label);
    if let Some(name) = name {
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &name));
    }
}

#[cfg(test)]
#[path = "shell_output_tests.rs"]
mod tests;
