//! The Sampling section of the Indexes tab: which databases the usage sampler leaves out. A
//! disclosure button in the toolbar row keeps it out of the way (it costs no row while closed);
//! opened, the excluded databases with a button each to sample them again, and a box to exclude
//! another. Draws the settings it is given and reports what the person did; whether it is open
//! and the typed text are the only things it keeps (in egui's memory).
use crate::I18n;
use crate::app::sampling_settings::SamplingSettings;
use crate::theme::{
    compact_text_button, gate, icons, outlined_field, text_button_with_icon, tonal_button,
};
use crate::viewmodel::IndexesAction;
use eframe::egui;

const INPUT_WIDTH: f32 = 200.0;

fn open_id() -> egui::Id {
    egui::Id::new("indexes_sampling_open")
}

/// The disclosure button, named by the section's heading and reported with its expanded state;
/// a chevron shows the state too. Returns whether the section is open after this frame's click.
pub fn sampling_toggle(ui: &mut egui::Ui, i18n: &I18n) -> bool {
    let was_open = ui.data(|d| d.get_temp::<bool>(open_id())).unwrap_or(false);
    let icon = match was_open {
        true => icons::CHEVRON_DOWN,
        false => icons::CHEVRON_RIGHT,
    };
    let response = text_button_with_icon(ui, icon, &i18n.t("indexes.sampling.heading"));
    let open = was_open != response.clicked();
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_expanded(open));
    ui.data_mut(|d| d.insert_temp(open_id(), open));
    open
}

/// The section's body (drawn while it is open): the list and the input.
pub fn show_sampling_body(
    ui: &mut egui::Ui,
    sampling: &SamplingSettings,
    i18n: &I18n,
) -> Option<IndexesAction> {
    body(ui, sampling, i18n)
}

fn body(ui: &mut egui::Ui, sampling: &SamplingSettings, i18n: &I18n) -> Option<IndexesAction> {
    let mut action = None;
    ui.weak(i18n.t("indexes.sampling.built_in"));
    if sampling.excluded_dbs.is_empty() {
        ui.label(i18n.t("indexes.sampling.none"));
    } else {
        ui.strong(i18n.t("indexes.sampling.excluded_heading"));
        for name in &sampling.excluded_dbs {
            action = excluded_row(ui, name, i18n).or(action.take());
        }
    }
    add_row(ui, i18n).or(action)
}

/// One excluded database and the button that samples it again. The button reads "Remove"; a
/// screen reader hears which database it is for.
fn excluded_row(ui: &mut egui::Ui, name: &str, i18n: &I18n) -> Option<IndexesAction> {
    ui.horizontal(|ui| {
        ui.label(name);
        let remove = compact_text_button(ui, i18n.t("indexes.sampling.remove"));
        let named = i18n.t_with("indexes.sampling.include_named", &[("name", name)]);
        remove.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, named.clone())
        });
        remove
            .clicked()
            .then(|| IndexesAction::IncludeDatabase(name.to_owned()))
    })
    .inner
}

/// The text box and its Exclude button (Enter in the box does the same). Text with nothing but
/// whitespace in it cannot be submitted.
fn add_row(ui: &mut egui::Ui, i18n: &I18n) -> Option<IndexesAction> {
    let draft_id = egui::Id::new("indexes_sampling_draft");
    ui.horizontal(|ui| {
        let mut draft: String = ui.data_mut(|d| d.get_temp(draft_id)).unwrap_or_default();
        let response = ui
            .vertical(|ui| {
                ui.set_width(INPUT_WIDTH);
                outlined_field(ui, &i18n.t("indexes.sampling.input"), &mut draft, "")
            })
            .inner;
        let can_add = SamplingSettings::normalized(&draft).is_some();
        let pressed_enter =
            response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        let clicked = gate(ui, can_add, |ui| {
            tonal_button(ui, i18n.t("indexes.sampling.add"))
        })
        .clicked();
        let submitted = can_add && (clicked || pressed_enter);
        let action = submitted.then(|| IndexesAction::ExcludeDatabase(draft.clone()));
        if submitted {
            draft.clear();
        }
        ui.data_mut(|d| d.insert_temp(draft_id, draft));
        action
    })
    .inner
}
