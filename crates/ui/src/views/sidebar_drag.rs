//! Drag and drop in the sidebar: a row can be dragged onto a database or connection (or a
//! collection, for documents). A thin layer over `AppState::drag_payload` and `drop_verdict`,
//! which decide everything; this only reads egui's drag state and draws. Menus and keys stay the
//! primary way: nothing here is needed to Copy and Paste.
use crate::I18n;
use crate::viewmodel::{CopyClipboard, DropVerdict, NodeRef, PasteTarget, SidebarAction};
use crate::views::sidebar_copy::TreeCtx;
use eframe::egui;

/// A selectable row that can also be dragged. Without a `ctx` (no selection model) it is a
/// plain selectable label.
pub fn row_button<'a>(
    ui: &mut egui::Ui,
    ctx: Option<&TreeCtx>,
    selected: bool,
    text: impl egui::IntoAtoms<'a>,
) -> egui::Response {
    if ctx.is_none() {
        return ui.selectable_label(selected, text);
    }
    let button = egui::Button::selectable(selected, text).sense(egui::Sense::click_and_drag());
    ui.add(button)
}

/// Starts a drag from `response`: it carries what the Copy menu would copy if `node` were
/// right-clicked (the whole selection when `node` is in it, else `node` alone). The clipboard is
/// not touched; it only changes if the drop is accepted.
pub fn start_drag(response: &egui::Response, ctx: Option<&TreeCtx>, node: &NodeRef) {
    let Some(ctx) = ctx else { return };
    if !response.drag_started() {
        return;
    }
    if let Some(payload) = ctx.state.drag_payload(node) {
        response.dnd_set_drag_payload(payload);
    }
}

/// `response` as a place to drop onto `target`. While something is dragged, a target that takes
/// it is outlined (strongly under the pointer); one that refuses it shows the reason as a tooltip
/// under the pointer. Releasing over a target that takes it returns the drop; releasing over any
/// other does nothing.
pub fn drop_target(
    response: &egui::Response,
    ctx: Option<&TreeCtx>,
    target: PasteTarget,
    i18n: &I18n,
) -> Option<SidebarAction> {
    let ctx = ctx?;
    let payload = egui::DragAndDrop::payload::<CopyClipboard>(&response.ctx)?;
    let over = response.contains_pointer();
    match ctx.state.drop_verdict(&payload, &target) {
        DropVerdict::Accept => {
            outline(response, over);
            let released = over.then(|| response.dnd_release_payload::<CopyClipboard>());
            released.flatten().map(|clipboard| SidebarAction::Drop {
                clipboard: Box::new((*clipboard).clone()),
                target,
            })
        }
        DropVerdict::Blocked(reason) => {
            if over {
                refuse(response, &i18n.t(reason));
            }
            None
        }
    }
}

/// Marks a row that takes the drag; `strong` while the pointer is over it.
fn outline(response: &egui::Response, strong: bool) {
    let visuals = &response.ctx.style().visuals;
    let stroke = if strong {
        visuals.selection.stroke
    } else {
        egui::Stroke::new(1.0, visuals.weak_text_color())
    };
    let painter = response.ctx.layer_painter(response.layer_id);
    painter.rect_stroke(response.rect, 2.0, stroke, egui::StrokeKind::Inside);
    if strong {
        response.ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    }
}

/// Says under the pointer why the row under it cannot take the drag.
fn refuse(response: &egui::Response, reason: &str) {
    response.ctx.set_cursor_icon(egui::CursorIcon::NoDrop);
    egui::Tooltip::always_open(
        response.ctx.clone(),
        response.layer_id,
        response.id,
        egui::PopupAnchor::Pointer,
    )
    .show(|ui| ui.label(reason));
}

#[cfg(test)]
#[path = "sidebar_drag_tests.rs"]
mod tests;
