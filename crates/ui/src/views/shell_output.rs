//! The Shell tab's output pane: a named region of virtualised, selectable monospace lines.
//! Stderr is drawn in the error color AND marked in words (a visible marker, and an accessible
//! name that says it is error output), so color is never the only signal. While a run is going
//! the pane follows the newest line unless the user scrolled away from the bottom.
use super::metrics::{fixed_row_sized, row_height};
use crate::I18n;
use crate::theme::current_ui;
use crate::viewmodel::shell::{OutputLine, ShellState};
use curlybrace_core::shell::Stream;
use eframe::egui::{self, accesskit};
use std::sync::Arc;

const REGION_ID: &str = "shell_output_region";

/// The output heading, the dropped-lines note and the lines (or what to expect when there are
/// none yet), all inside one region named for assistive technology.
pub(crate) fn show_output(ui: &mut egui::Ui, state: &ShellState, i18n: &I18n) {
    let ctx = ui.ctx().clone();
    let region = egui::Id::new(REGION_ID);
    let name = i18n.t("shell.output.region");
    ctx.accesskit_node_builder(region, |node| {
        node.set_role(accesskit::Role::Region);
        node.set_label(name);
    });
    let rect = ctx.with_accessibility_parent(region, || {
        ui.scope(|ui| {
            ui.strong(i18n.t("shell.output.title"));
            if state.dropped_lines > 0 {
                let count = state.dropped_lines.to_string();
                ui.label(i18n.t_with("shell.output.dropped", &[("count", &count)]));
            }
            if state.line_count() == 0 {
                ui.label(i18n.t(empty_key(state)));
            } else {
                show_lines(ui, state, i18n);
            }
        })
        .response
        .rect
    });
    ctx.accesskit_node_builder(region, |node| {
        node.set_bounds(accesskit::Rect {
            x0: rect.min.x.into(),
            y0: rect.min.y.into(),
            x1: rect.max.x.into(),
            y1: rect.max.y.into(),
        });
    });
}

fn empty_key(state: &ShellState) -> &'static str {
    match state.is_running() {
        true => "shell.output.waiting",
        false => "shell.output.empty",
    }
}

fn show_lines(ui: &mut egui::Ui, state: &ShellState, i18n: &I18n) {
    let height = row_height(ui);
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let normal = ui.visuals().text_color();
    let error = current_ui(ui).error;
    let marker = i18n.t("shell.output.stderr_marker");
    let truncated = i18n.t("shell.output.truncated");
    egui::ScrollArea::both()
        .id_salt("shell_output")
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
                        // The marker is part of the line's accessible name below.
                        ui.add(egui::Label::new(egui::WidgetText::Galley(mark)).selectable(false));
                    }
                    line_label(ui, galley, name);
                });
            }
        });
}

/// The line as drawn: an over-long line ends with the "cut" marker.
fn shown_text(line: &OutputLine, truncated: &str) -> String {
    match line.truncated {
        true => format!("{}{truncated}", line.text),
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
