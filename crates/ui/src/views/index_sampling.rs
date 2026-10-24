//! The Sampling section of the Indexes tab: which databases the usage sampler leaves out. A
//! collapsed header keeps it out of the way; inside, the excluded databases with a button each to
//! sample them again, and a box to exclude another. Draws the settings it is given and reports
//! what the person did; the typed text is the only thing it keeps (in egui's memory).
use crate::I18n;
use crate::app::sampling_settings::SamplingSettings;
use crate::viewmodel::IndexesAction;
use eframe::egui;

const INPUT_WIDTH: f32 = 200.0;

/// The header and, when opened, the list and the input.
pub fn show_sampling(
    ui: &mut egui::Ui,
    sampling: &SamplingSettings,
    i18n: &I18n,
) -> Option<IndexesAction> {
    let mut action = None;
    egui::CollapsingHeader::new(i18n.t("indexes.sampling.heading"))
        .id_salt("indexes_sampling")
        .default_open(false)
        .show(ui, |ui| action = body(ui, sampling, i18n));
    action
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
        let remove = ui.button(i18n.t("indexes.sampling.remove"));
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
        let caption = ui.label(i18n.t("indexes.sampling.input"));
        let response = ui
            .add(egui::TextEdit::singleline(&mut draft).desired_width(INPUT_WIDTH))
            .labelled_by(caption.id);
        let can_add = SamplingSettings::normalized(&draft).is_some();
        let pressed_enter =
            response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        let clicked = ui
            .add_enabled(can_add, egui::Button::new(i18n.t("indexes.sampling.add")))
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
