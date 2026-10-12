//! The collapsible "Rules" section: the five rule controls.
use super::setup::text_field;
use crate::I18n;
use crate::viewmodel::compare::{RulesForm, ScopeKind};
use eframe::egui;

/// Returns the edited rules when the user changed any control this frame.
pub fn show_rules(ui: &mut egui::Ui, rules: &RulesForm, i18n: &I18n) -> Option<RulesForm> {
    let mut edited = rules.clone();
    egui::CollapsingHeader::new(i18n.t("compare.rules.title"))
        .id_salt("compare_rules")
        .default_open(false)
        .show(ui, |ui| {
            edited = rule_controls(ui, edited.clone(), i18n);
        });
    (edited != *rules).then_some(edited)
}

fn rule_controls(ui: &mut egui::Ui, rules: RulesForm, i18n: &I18n) -> RulesForm {
    let scope = scope_combo(ui, rules.scope, i18n);
    let paths = match scope {
        ScopeKind::All => rules.paths_text.clone(),
        _ => {
            let keys = ("compare.rules.paths", "compare.rules.paths_hint");
            text_field(ui, "paths", keys, &rules.paths_text, i18n)
                .unwrap_or(rules.paths_text.clone())
        }
    };
    let mut next = RulesForm {
        scope,
        paths_text: paths,
        ..rules
    };
    let flags = [
        (
            (
                "compare.rules.ignore_array_order",
                "compare.rules.ignore_array_order_hint",
            ),
            &mut next.ignore_array_order,
        ),
        (
            (
                "compare.rules.numbers_equal_across_types",
                "compare.rules.numbers_equal_across_types_hint",
            ),
            &mut next.numbers_equal_across_types,
        ),
        (
            (
                "compare.rules.ignore_field_order",
                "compare.rules.ignore_field_order_hint",
            ),
            &mut next.ignore_field_order,
        ),
    ];
    for (keys, flag) in flags {
        flag_checkbox(ui, keys, flag, i18n);
    }
    next.include_id = include_id_combo(ui, next.include_id, i18n);
    next
}

fn flag_checkbox(ui: &mut egui::Ui, keys: (&str, &str), flag: &mut bool, i18n: &I18n) {
    ui.push_id(keys.0, |ui| {
        ui.checkbox(flag, i18n.t(keys.0))
            .on_hover_text(i18n.t(keys.1));
    });
}

fn scope_key(scope: ScopeKind) -> &'static str {
    match scope {
        ScopeKind::All => "compare.rules.scope_all",
        ScopeKind::AllExcept => "compare.rules.scope_all_except",
        ScopeKind::Only => "compare.rules.scope_only",
    }
}

fn scope_combo(ui: &mut egui::Ui, current: ScopeKind, i18n: &I18n) -> ScopeKind {
    let mut scope = current;
    egui::ComboBox::new("scope", i18n.t("compare.rules.scope"))
        .selected_text(i18n.t(scope_key(current)))
        .show_ui(ui, |ui| {
            for option in [ScopeKind::All, ScopeKind::AllExcept, ScopeKind::Only] {
                let label = i18n.t(scope_key(option));
                if ui.selectable_label(current == option, label).clicked() {
                    scope = option;
                }
            }
        });
    scope
}

fn include_id_key(value: Option<bool>) -> &'static str {
    match value {
        None => "compare.rules.include_id_auto",
        Some(true) => "compare.rules.include_id_yes",
        Some(false) => "compare.rules.include_id_no",
    }
}

fn include_id_combo(ui: &mut egui::Ui, current: Option<bool>, i18n: &I18n) -> Option<bool> {
    let mut value = current;
    egui::ComboBox::new("include_id", i18n.t("compare.rules.include_id"))
        .selected_text(i18n.t(include_id_key(current)))
        .show_ui(ui, |ui| {
            for option in [None, Some(true), Some(false)] {
                let label = i18n.t(include_id_key(option));
                if ui.selectable_label(current == option, label).clicked() {
                    value = option;
                }
            }
        });
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(rules: RulesForm) -> (RulesForm, Vec<String>) {
        let ctx = egui::Context::default();
        let i18n = I18n::english();
        let mut next = rules.clone();
        let mut texts = Vec::new();
        for _ in 0..2 {
            texts.clear();
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    next = rule_controls(ui, rules.clone(), &i18n);
                });
            });
            for clipped in &output.shapes {
                if let egui::Shape::Text(t) = &clipped.shape {
                    texts.push(t.galley.text().to_string());
                }
            }
        }
        (next, texts)
    }

    #[test]
    fn every_scope_renders_and_an_idle_frame_changes_nothing() {
        for scope in [ScopeKind::All, ScopeKind::AllExcept, ScopeKind::Only] {
            let rules = RulesForm {
                scope,
                paths_text: "a.b".into(),
                ..RulesForm::default()
            };
            let (next, texts) = render(rules.clone());
            assert_eq!(next, rules);
            assert!(texts.iter().all(|t| !t.contains('⟦')), "{texts:?}");
            assert!(texts.iter().any(|t| t == "Ignore array order"));
        }
    }

    #[test]
    fn the_paths_box_only_appears_when_some_fields_are_chosen() {
        let (_, all) = render(RulesForm::default());
        assert!(!all.iter().any(|t| t == "Field names"));
        let only = RulesForm {
            scope: ScopeKind::Only,
            ..RulesForm::default()
        };
        let (_, texts) = render(only);
        assert!(texts.iter().any(|t| t == "Field names"));
    }
}
