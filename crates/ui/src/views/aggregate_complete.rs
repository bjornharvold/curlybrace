//! A stage editor's completion list: the Shell tab's popup (opening, keys, placement, picking)
//! fed with the names of [`complete_stage`]. One editor, one list, kept in egui temp data.
use super::shell_complete::{
    Moment, Popup, applied, clicking_popup, next_popup_with, put_pick, read_keys, show_popup,
    typed_trigger,
};
use super::shell_editor::caret_of;
use crate::I18n;
use crate::viewmodel::stage_complete::complete_stage;
use eframe::egui;

/// Where the editor remembers its open list.
const POPUP: &str = "complete";

/// A multi-line monospace text box with id `id` over `buffer`, `rows` tall, with the stage-name
/// list. Returns the box's response and whether `buffer` changed (typed in or a name picked).
pub(super) fn completing_editor(
    ui: &mut egui::Ui,
    id: egui::Id,
    buffer: &mut String,
    rows: usize,
    i18n: &I18n,
) -> (egui::Response, bool) {
    let popup = ui.data(|d| d.get_temp::<Popup>(id.with(POPUP)));
    let keys = read_keys(ui, id, popup.as_ref());
    let popup = popup.map(|p| Popup {
        selected: keys.selected.unwrap_or(p.selected),
        ..p
    });
    let typed = ui.memory(|m| m.has_focus(id)) && typed_trigger(ui);
    let key_pick = popup
        .as_ref()
        .filter(|_| keys.pick)
        .map(|p| picked(ui.ctx(), id, buffer, p, p.selected));
    let popup = popup.filter(|_| !keys.pick && !keys.close);
    if let Some(next) = &key_pick {
        buffer.clone_from(next);
    }
    let output = egui::TextEdit::multiline(buffer)
        .id(id)
        .font(egui::TextStyle::Monospace)
        .desired_rows(rows)
        .desired_width(f32::INFINITY)
        .show(ui);
    let caret = caret_of(&output, buffer);
    let moment = Moment {
        text: buffer,
        caret: caret.map(|(byte, _)| byte),
        focused: ui.memory(|m| m.has_focus(id)) || clicking_popup(ui.ctx(), id),
        typed,
        forced: keys.forced,
        changed: output.response.changed(),
    };
    let names = |text: &str, at: usize, forced: bool| complete_stage(text, at, forced);
    let popup = next_popup_with(popup, &moment, &names);
    let click_pick = popup.as_ref().zip(caret).and_then(|(p, (_, rect))| {
        let index = show_popup(ui, id, rect, p, i18n)?;
        ui.memory_mut(|m| m.request_focus(id));
        Some(picked(ui.ctx(), id, buffer, p, index))
    });
    let open = popup.filter(|_| click_pick.is_none());
    ui.data_mut(|d| match open {
        Some(open) => d.insert_temp(id.with(POPUP), open),
        None => d.remove::<Popup>(id.with(POPUP)),
    });
    let changed = output.response.changed() || key_pick.is_some() || click_pick.is_some();
    if let Some(next) = click_pick {
        *buffer = next;
    }
    (output.response, changed)
}

/// `text` with item `index` of `popup` picked, the caret put after the insertion.
fn picked(ctx: &egui::Context, id: egui::Id, text: &str, popup: &Popup, index: usize) -> String {
    let item = &popup.completion.items[index];
    let (next, caret) = applied(text, &popup.completion.replace, &item.insert);
    put_pick(ctx, id, text, &next, caret);
    next
}
