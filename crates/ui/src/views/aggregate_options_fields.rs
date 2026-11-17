//! The controls of the Options section: a text field that applies when it is committed, and a
//! combo that picks one of a few labelled values. Every control is named by its label, and a
//! problem shows in words under the field it belongs to.
use crate::I18n;
use crate::theme::{FieldOptions, outlined_field_opts};
use crate::viewmodel::aggregate_options::Tri;
use eframe::egui::{self, WidgetInfo, WidgetType};

/// What a text field did this frame: the new text, and whether it was committed (Enter, or the
/// focus left it).
pub(super) struct Typed {
    pub text: Option<String>,
    pub commit: bool,
}

/// A single-line field named `label`, with `problem` (already in words) under it.
pub(super) fn typed_field(
    ui: &mut egui::Ui,
    label: &str,
    text: &str,
    hint: &str,
    problem: Option<String>,
) -> Typed {
    let mut buffer = text.to_string();
    // A fixed id: the field keeps its focus when the controls around it change (the index list
    // arrives while the hint is being typed).
    let options = FieldOptions {
        error: problem.as_deref(),
        id: Some(egui::Id::new(("aggregate_option_field", label))),
        ..FieldOptions::default()
    };
    let response = outlined_field_opts(ui, label, &mut buffer, hint, options);
    Typed {
        text: response.changed().then_some(buffer),
        commit: response.lost_focus(),
    }
}

/// A combo named `label` showing `shown`, listing `choices`; the one picked this frame. The
/// label is drawn above it.
pub(super) fn pick_combo<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    current: T,
    choices: &[(T, String)],
) -> Option<T> {
    let shown = choices
        .iter()
        .find(|(value, _)| *value == current)
        .map(|(_, text)| text.clone())
        .unwrap_or_default();
    let mut picked = None;
    ui.label(label);
    let combo = egui::ComboBox::from_id_salt(("aggregate_option", id))
        .selected_text(shown)
        .show_ui(ui, |ui| {
            for (value, text) in choices {
                if ui.selectable_label(*value == current, text).clicked() {
                    picked = Some(*value);
                }
            }
        });
    combo
        .response
        .widget_info(|| WidgetInfo::labeled(WidgetType::ComboBox, true, label));
    picked
}

/// The key of a tri-state's choice.
fn tri_key(value: Tri) -> &'static str {
    match value {
        Tri::Default => "aggregate.options.tri.default",
        Tri::On => "aggregate.options.tri.on",
        Tri::Off => "aggregate.options.tri.off",
    }
}

/// A Server default | On | Off combo named `label`.
pub(super) fn tri_combo(
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    value: Tri,
    i18n: &I18n,
) -> Option<Tri> {
    let choices: Vec<(Tri, String)> = [Tri::Default, Tri::On, Tri::Off]
        .into_iter()
        .map(|v| (v, i18n.t(tri_key(v))))
        .collect();
    pick_combo(ui, id, label, value, &choices)
}
