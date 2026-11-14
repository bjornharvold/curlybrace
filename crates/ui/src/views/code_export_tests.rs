//! The Copy as code dialog by accessible name and role: every control named, the nine languages in
//! their order, an unusable name shown in place of the code with Copy off, Enter copying and
//! Escape closing, and what lands on the clipboard being the previewed text.
use super::*;
use crate::theme;
use crate::viewmodel::code_export::CodeExportUi;
use curlybrace_core::codegen::CodeRequest;
use curlybrace_core::query::QueryText;
use curlybrace_core::store::{Namespace, Query};
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

const BIG: (f32, f32) = (1000.0, 700.0);
const SMALL: (f32, f32) = (800.0, 500.0);

struct Frame {
    code: CodeExportUi,
    seen: Vec<CodeAction>,
    copied: Vec<String>,
}

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn find_request() -> CodeRequest {
    CodeRequest::Find {
        ns: Namespace::new("crm", "customers"),
        query: Query {
            filter: bson::doc! { "status": "active" },
            ..Query::default()
        },
    }
}

fn aggregate_request() -> CodeRequest {
    CodeRequest::Aggregate {
        ns: Namespace::new("crm", "customers"),
        stages: vec![
            bson::doc! { "$match": { "status": "active" } },
            bson::doc! { "$out": "totals" },
        ],
    }
}

/// Applies an action to the dialog's state as the reducer does.
fn apply(code: CodeExportUi, action: &CodeAction) -> CodeExportUi {
    match action {
        CodeAction::SetLanguage(language) => code.language_set(*language),
        CodeAction::SetSetup(setup) => code.setup_set(*setup),
        CodeAction::SetName(field, name) => code.name_set(*field, name.clone()),
        CodeAction::SetWriteStage(keep) => code.write_stage_set(*keep),
        CodeAction::Close => code.closed(),
        CodeAction::Open(_) | CodeAction::Copy => code,
    }
}

fn harness(request: CodeRequest, size: (f32, f32)) -> Harness<'static, Frame> {
    let frame = Frame {
        code: CodeExportUi::default().opened(request),
        seen: Vec::new(),
        copied: Vec::new(),
    };
    let mut installed = false;
    let mut h = Harness::builder()
        .with_size(egui::vec2(size.0, size.1))
        .build_ui_state(
            move |ui, f: &mut Frame| {
                if !installed {
                    theme::install(ui.ctx());
                    installed = true;
                }
                let actions = show_code_dialog(ui.ctx(), &f.code, &I18n::english());
                let copied = ui.ctx().output(|o| {
                    o.commands
                        .iter()
                        .filter_map(|c| match c {
                            egui::OutputCommand::CopyText(text) => Some(text.clone()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                });
                f.copied.extend(copied);
                for action in actions {
                    f.code = apply(f.code.clone(), &action);
                    f.seen.push(action);
                }
            },
            frame,
        );
    h.run_steps(4);
    h
}

fn focused(h: &Harness<'static, Frame>) -> Option<String> {
    h.root()
        .children_recursive()
        .find(|n| n.accesskit_node().is_focused())
        .and_then(|n| n.accesskit_node().label())
}

#[test]
fn every_control_has_a_role_and_a_name() {
    let h = harness(aggregate_request(), BIG);
    h.get_by_label(&t("codegen.title"));
    h.get_by_role_and_label(Role::ComboBox, &t("codegen.language.label"));
    h.get_by_role_and_label(Role::CheckBox, &t("codegen.setup"));
    h.get_by_role_and_label(Role::CheckBox, &t("codegen.write_stage"));
    for key in [
        "codegen.name.client",
        "codegen.name.db",
        "codegen.name.collection",
    ] {
        h.get_by_role_and_label(Role::TextInput, &t(key));
    }
    h.get_by_role_and_label(Role::Button, &t("codegen.copy"));
    h.get_by_role_and_label(Role::Button, &t("common.close"));
    h.get_by_role_and_label(Role::MultilineTextInput, &t("codegen.preview"));
}

#[test]
fn a_find_has_no_write_stage_option() {
    let h = harness(find_request(), BIG);
    let label = t("codegen.write_stage");
    assert!(h.query_by_role_and_label(Role::CheckBox, &label).is_none());
}

#[test]
fn the_preview_is_the_generated_text_and_read_only() {
    let h = harness(find_request(), BIG);
    let code = h.state().code.clipboard_text().unwrap().to_string();
    let name = t("codegen.preview");
    let preview = h.get_by_role_and_label(Role::MultilineTextInput, &name);
    assert_eq!(preview.accesskit_node().value(), Some(code));
}

#[test]
fn focus_starts_on_the_language_picker() {
    let h = harness(find_request(), BIG);
    assert_eq!(focused(&h), Some(t("codegen.language.label")));
}

#[test]
fn the_picker_lists_the_nine_languages_in_order() {
    let mut h = harness(find_request(), BIG);
    h.get_by_role_and_label(Role::ComboBox, &t("codegen.language.label"))
        .click();
    h.run_steps(3);
    let names: Vec<String> = [
        "mongosh", "node", "python", "java", "csharp", "go", "php", "ruby", "rust",
    ]
    .iter()
    .map(|key| t(&format!("codegen.language.{key}")))
    .collect();
    let mut tops = Vec::new();
    for name in &names {
        let node = h
            .query_all_by_label(name)
            .last()
            .unwrap_or_else(|| panic!("{name} is not in the list"));
        tops.push(node.rect().top());
    }
    assert!(tops.windows(2).all(|w| w[0] < w[1]), "{tops:?}");
    h.get_by_label(&names[3]).click();
    h.run_steps(3);
    assert_eq!(h.state().seen, [CodeAction::SetLanguage(Language::Java)]);
}

#[test]
fn the_options_are_actions() {
    let mut h = harness(aggregate_request(), BIG);
    h.get_by_role_and_label(Role::CheckBox, &t("codegen.setup"))
        .click();
    h.run_steps(4);
    let write_label = t("codegen.write_stage");
    h.get_by_role_and_label(Role::CheckBox, &write_label)
        .click();
    h.run_steps(4);
    assert_eq!(
        h.state().seen,
        [
            CodeAction::SetSetup(false),
            CodeAction::SetWriteStage(false)
        ]
    );
}

#[test]
fn an_unusable_name_replaces_the_code_and_turns_copy_off() {
    let mut h = harness(find_request(), BIG);
    let db_label = t("codegen.name.db");
    h.get_by_role_and_label(Role::TextInput, &db_label).focus();
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, &t("codegen.name.db"))
        .type_text(" bad");
    h.run_steps(3);
    let message = I18n::english().t_with(
        "codegen.error.invalid_name",
        &[("field", &t("codegen.name.db"))],
    );
    h.get_by_label(&message);
    let preview_label = t("codegen.preview");
    let preview = h.query_by_role_and_label(Role::MultilineTextInput, &preview_label);
    assert!(preview.is_none(), "the error is shown in place of the code");
    let copy_label = t("codegen.copy");
    let copy = h.get_by_role_and_label(Role::Button, &copy_label);
    assert!(copy.accesskit_node().is_disabled());
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert!(h.state().copied.is_empty());
}

#[test]
fn copy_puts_exactly_the_previewed_text_on_the_clipboard() {
    let mut h = harness(aggregate_request(), BIG);
    let shown = h.state().code.clipboard_text().unwrap().to_string();
    h.get_by_role_and_label(Role::Button, &t("codegen.copy"))
        .click();
    h.run_steps(3);
    assert_eq!(h.state().copied, [shown]);
    assert_eq!(h.state().seen, [CodeAction::Copy]);
    assert!(h.state().code.dialog.is_some(), "the dialog stays open");
}

#[test]
fn enter_copies_and_escape_closes() {
    let mut h = harness(find_request(), BIG);
    let shown = h.state().code.clipboard_text().unwrap().to_string();
    // Focus rests on the language picker, which Enter belongs to: move to a name field first.
    h.get_by_role_and_label(Role::TextInput, &t("codegen.name.client"))
        .focus();
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().copied, [shown]);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.state().seen.contains(&CodeAction::Close));
    assert!(h.state().code.dialog.is_none());
}

/// The text a language previews, as the dialog generates it.
fn preview_of(request: CodeRequest, language: Language) -> String {
    CodeExportUi::default()
        .opened(request)
        .language_set(language)
        .clipboard_text()
        .unwrap()
        .to_string()
}

#[test]
fn enter_choosing_a_language_does_not_copy_the_previous_one() {
    let mut h = harness(find_request(), BIG);
    // Open the picker from the keyboard, step to the next language and choose it with Enter.
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    h.key_press(egui::Key::ArrowDown);
    h.run_steps(3);
    h.key_press(egui::Key::ArrowDown);
    h.run_steps(3);
    h.key_press(egui::Key::Enter);
    h.run_steps(4);
    let chosen = h
        .state()
        .seen
        .iter()
        .find_map(|a| match a {
            CodeAction::SetLanguage(l) => Some(*l),
            _ => None,
        })
        .expect("a language was chosen from the keyboard");
    assert!(
        !h.state().seen.contains(&CodeAction::Copy),
        "choosing must not copy: {:?}",
        h.state().seen
    );
    assert!(h.state().copied.is_empty());
    // Enter on a name field now copies the chosen language's preview.
    h.get_by_role_and_label(Role::TextInput, &t("codegen.name.client"))
        .focus();
    h.run_steps(4);
    assert_ne!(
        chosen,
        Language::Mongosh,
        "the walk moved off the first item"
    );
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().copied, [preview_of(find_request(), chosen)]);
}

#[test]
fn enter_on_close_closes_without_copying() {
    let mut h = harness(find_request(), BIG);
    h.get_by_role_and_label(Role::Button, &t("common.close"))
        .focus();
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(h.state().copied.is_empty());
    assert!(h.state().code.dialog.is_none());
}

#[test]
fn it_fits_the_small_window_with_the_buttons_in_view() {
    let h = harness(aggregate_request(), SMALL);
    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SMALL.0, SMALL.1));
    for (role, key) in [
        (Role::Button, "codegen.copy"),
        (Role::Button, "common.close"),
    ] {
        let rect = h.get_by_role_and_label(role, &t(key)).rect();
        assert!(window.contains_rect(rect), "{key}: {rect:?}");
    }
}

#[test]
fn nothing_is_drawn_without_a_dialog() {
    let mut h = harness(find_request(), BIG);
    h.state_mut().code = CodeExportUi::default();
    h.run_steps(2);
    let label = t("codegen.copy");
    assert!(h.query_by_role_and_label(Role::Button, &label).is_none());
    let _ = QueryText::default();
}
