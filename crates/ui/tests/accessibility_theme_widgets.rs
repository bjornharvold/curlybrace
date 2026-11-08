//! The theme's custom widgets expose the role, label, value and selection state of the egui
//! widgets they replace, and every interactive one is reachable by Tab and draws a visible focus
//! ring. Fast, offline, no GPU.
#[path = "support/widget_gallery.rs"]
mod widget_gallery;

use curlybrace_ui::theme::{Mode, Tone, chip, scheme};
use eframe::egui::{
    self, Rect, Shape,
    accesskit::{Invalid, Role, Toggled},
};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use std::collections::BTreeSet;
use widget_gallery::{GalleryState, TAB_ORDER, interactive, themed};

fn harness() -> Harness<'static, GalleryState> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 500.0))
        .build_ui_state(
            themed(Mode::Light, |ui, state: &mut GalleryState| {
                interactive(ui, state);
                chip(ui, Tone::Good, "Uses index");
            }),
            GalleryState::default(),
        );
    harness.run();
    harness
}

#[test]
fn widgets_are_found_by_role_and_label() {
    let harness = harness();
    harness.get_by_role_and_label(Role::RadioButton, "Browse");
    harness.get_by_role_and_label(Role::RadioButton, "Compare");
    harness.get_by_role_and_label(Role::Button, "customers");
    harness.get_by_role_and_label(Role::Label, "Uses index");
    for button in ["Find", "Clear", "Cancel"] {
        harness.get_by_role_and_label(Role::Button, button);
    }
    let field = harness.get_by_role_and_label(Role::TextInput, "Name");
    assert_eq!(field.value().as_deref(), Some("Production"));
}

fn toggled(harness: &Harness<'_, GalleryState>, role: Role, label: &str) -> Option<Toggled> {
    harness
        .get_by_role_and_label(role, label)
        .accesskit_node()
        .toggled()
}

#[test]
fn segmented_and_pills_expose_their_selection() {
    let mut harness = harness();
    assert_eq!(
        toggled(&harness, Role::RadioButton, "Browse"),
        Some(Toggled::True)
    );
    assert_eq!(
        toggled(&harness, Role::RadioButton, "Compare"),
        Some(Toggled::False)
    );
    assert_eq!(
        toggled(&harness, Role::Button, "customers"),
        Some(Toggled::True)
    );
    harness.get_by_label("Compare").click();
    harness.get_by_label("invoices").click();
    harness.run();
    assert_eq!(harness.state().view, 1);
    assert_eq!(harness.state().collection, 1);
    assert_eq!(
        toggled(&harness, Role::RadioButton, "Compare"),
        Some(Toggled::True)
    );
    assert_eq!(
        toggled(&harness, Role::Button, "invoices"),
        Some(Toggled::True)
    );
}

#[test]
fn disabled_buttons_are_marked_disabled() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(300.0, 100.0))
        .build_ui(|ui| {
            ui.add_enabled_ui(false, |ui| {
                curlybrace_ui::theme::filled_button(ui, "Save");
            });
        });
    harness.run();
    assert!(
        harness
            .get_by_role_and_label(Role::Button, "Save")
            .accesskit_node()
            .is_disabled()
    );
}

/// Rectangles stroked with `color` at `width` or more in the last frame.
fn ring_rects(harness: &Harness<'_, GalleryState>, color: egui::Color32) -> Vec<Rect> {
    harness
        .output()
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            Shape::Rect(r) if r.stroke.color == color && r.stroke.width >= 2.0 => Some(r.rect),
            _ => None,
        })
        .collect()
}

/// The label of the focused node, if any.
fn focused(harness: &Harness<'_, GalleryState>) -> Option<(String, Rect)> {
    harness
        .root()
        .children_recursive()
        .find(|n| n.is_focused())
        .map(|n| (n.accesskit_node().label().unwrap_or_default(), n.rect()))
}

#[test]
fn custom_widgets_are_reachable_by_tab_and_show_focus() {
    let primary = scheme(Mode::Light).primary;
    let mut harness = harness();
    assert!(
        ring_rects(&harness, primary).is_empty(),
        "no ring before focus"
    );
    let mut reached = Vec::new();
    for _ in 0..TAB_ORDER.len() * 2 {
        harness.key_press(egui::Key::Tab);
        harness.run();
        let Some((label, rect)) = focused(&harness) else {
            continue;
        };
        let rings = ring_rects(&harness, primary);
        assert!(
            rings
                .iter()
                .any(|r| r.expand(1.0).contains_rect(rect) && rect.expand(4.0).contains_rect(*r)),
            "no 2 dp primary ring around focused {label:?} at {rect:?}; rings: {rings:?}"
        );
        if !reached.contains(&label) {
            reached.push(label);
        }
    }
    let reached: BTreeSet<&str> = reached.iter().map(String::as_str).collect();
    let expected: BTreeSet<&str> = TAB_ORDER.iter().copied().collect();
    assert_eq!(reached, expected);
}

#[test]
fn outlined_field_error_is_invalid_with_the_message_as_description() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(400.0, 200.0))
        .build_ui_state(
            themed(Mode::Light, |ui, text: &mut String| {
                curlybrace_ui::theme::outlined_field_with(
                    ui,
                    "TTL",
                    text,
                    "seconds",
                    Some("Must be a number"),
                );
                curlybrace_ui::theme::outlined_field(ui, "Name", &mut String::new(), "e.g. crm");
            }),
            "abc".to_owned(),
        );
    harness.run();
    let field = harness.get_by_role_and_label(Role::TextInput, "TTL");
    let node = field.accesskit_node();
    assert_eq!(node.data().invalid(), Some(Invalid::True));
    assert_eq!(node.description().as_deref(), Some("Must be a number"));
    assert_eq!(node.placeholder(), Some("seconds"));
    assert_eq!(field.value().as_deref(), Some("abc"));
    let valid = harness.get_by_role_and_label(Role::TextInput, "Name");
    assert_eq!(valid.accesskit_node().data().invalid(), None);
    assert_eq!(valid.accesskit_node().placeholder(), Some("e.g. crm"));
}
