//! The Copy as code dialog in the accesskit tree, rendered with the REAL English locale at the
//! standard 1000x700 and the small 800x500 window: every control has a role and a name, the code
//! scrolls while Copy and Close stay in view, an unusable name shows its message in place of the
//! code with Copy off, and focus starts on the language picker. Offline, no GPU.
use curlybrace_core::codegen::{CodeOptions, CodeRequest, Language};
use curlybrace_core::store::Namespace;
use curlybrace_ui::I18n;
use curlybrace_ui::theme;
use curlybrace_ui::viewmodel::{CodeExportUi, NameField};
use curlybrace_ui::views::code_export::show_code_dialog;
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

const STANDARD: (f32, f32) = (1000.0, 700.0);
const SMALL: (f32, f32) = (800.0, 500.0);

fn t(key: &str) -> String {
    I18n::english().t(key)
}

/// A pipeline long enough that its code cannot fit the small window.
fn long_pipeline() -> CodeRequest {
    let stage = |n: i32| bson::doc! { "$match": { "field": n, "status": "active", "kind": "x" } };
    CodeRequest::Aggregate {
        ns: Namespace::new("crm", "customers"),
        stages: (0..30)
            .map(stage)
            .chain([bson::doc! { "$out": "totals" }])
            .collect(),
    }
}

fn open(code: CodeExportUi, size: (f32, f32)) -> Harness<'static, CodeExportUi> {
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(size.0, size.1))
        .build_ui_state(
            move |ui, code: &mut CodeExportUi| {
                if !installed {
                    theme::install(ui.ctx());
                    installed = true;
                }
                show_code_dialog(ui.ctx(), code, &I18n::english());
            },
            code,
        );
    harness.run_steps(4);
    harness
}

fn window(size: (f32, f32)) -> egui::Rect {
    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0, size.1))
}

fn rect_of(harness: &Harness<'static, CodeExportUi>, role: Role, label: &str) -> egui::Rect {
    harness.get_by_role_and_label(role, label).rect()
}

fn assert_inside(size: (f32, f32), name: &str, rect: egui::Rect) {
    assert!(
        rect.width() > 1.0 && rect.height() > 1.0,
        "{name}: {rect:?}"
    );
    assert!(
        window(size).contains_rect(rect),
        "{name} leaves {size:?}: {rect:?}"
    );
}

#[test]
fn every_control_is_named_and_inside_the_window_at_both_sizes() {
    for size in [STANDARD, SMALL] {
        let code = CodeExportUi::default().opened(long_pipeline());
        let harness = open(code, size);
        let controls = [
            (Role::ComboBox, t("codegen.language.label")),
            (Role::CheckBox, t("codegen.setup")),
            (Role::CheckBox, t("codegen.write_stage")),
            (Role::TextInput, t("codegen.name.client")),
            (Role::TextInput, t("codegen.name.db")),
            (Role::TextInput, t("codegen.name.collection")),
            (Role::Button, t("codegen.copy")),
            (Role::Button, t("common.close")),
        ];
        let mut rects = Vec::new();
        for (role, label) in &controls {
            let rect = rect_of(&harness, *role, label);
            assert_inside(size, label, rect);
            rects.push((label.clone(), rect));
        }
        for (i, (name_a, a)) in rects.iter().enumerate() {
            for (name_b, b) in &rects[i + 1..] {
                assert!(
                    !a.intersect(*b).is_positive(),
                    "{name_a} overlaps {name_b} at {size:?}"
                );
            }
        }
    }
}

#[test]
fn the_code_scrolls_and_the_buttons_stay_in_view_at_both_sizes() {
    for size in [STANDARD, SMALL] {
        let code = CodeExportUi::default().opened(long_pipeline());
        let harness = open(code, size);
        let preview = rect_of(&harness, Role::MultilineTextInput, &t("codegen.preview"));
        assert!(preview.width() > 100.0, "{preview:?}");
        let copy = rect_of(&harness, Role::Button, &t("codegen.copy"));
        let close = rect_of(&harness, Role::Button, &t("common.close"));
        assert_inside(size, "Copy", copy);
        assert_inside(size, "Close", close);
        // The text is taller than the window: the box scrolls, so the buttons are not pushed out.
        assert!(
            preview.height() > 400.0,
            "the whole text is laid out inside its scroll area"
        );
        assert!(preview.top() < copy.top(), "the code is above the buttons");
    }
}

#[test]
fn an_unusable_name_shows_its_message_in_place_of_the_code_and_turns_copy_off() {
    for size in [STANDARD, SMALL] {
        let code = CodeExportUi::default()
            .opened(long_pipeline())
            .name_set(NameField::Collection, "not valid".into());
        let harness = open(code, size);
        let message = I18n::english().t_with(
            "codegen.error.invalid_name",
            &[("field", &t("codegen.name.collection"))],
        );
        let shown = harness.get_by_label(&message);
        assert_inside(size, "error message", shown.rect());
        let preview = t("codegen.preview");
        assert!(
            harness
                .query_by_role_and_label(Role::MultilineTextInput, &preview)
                .is_none()
        );
        let copy_label = t("codegen.copy");
        let copy = harness.get_by_role_and_label(Role::Button, &copy_label);
        assert!(copy.accesskit_node().is_disabled());
        assert_inside(size, "Copy", copy.rect());
    }
}

#[test]
fn focus_starts_on_the_language_picker_and_the_language_is_shown() {
    let mut code = CodeExportUi::default().language_set(Language::Go);
    code.last = (Language::Go, CodeOptions::default());
    let code = code.opened(long_pipeline());
    let harness = open(code, STANDARD);
    let focused = harness
        .root()
        .children_recursive()
        .find(|n| n.accesskit_node().is_focused())
        .and_then(|n| n.accesskit_node().label());
    assert_eq!(focused, Some(t("codegen.language.label")));
    let picker_label = t("codegen.language.label");
    let picker = harness.get_by_role_and_label(Role::ComboBox, &picker_label);
    assert_eq!(
        picker.accesskit_node().value(),
        Some(t("codegen.language.go"))
    );
}
