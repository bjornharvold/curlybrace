//! Cross-widget behavior: nested [`gate`]s, the field height and the field character limit.
use super::widgets_tests::themed;
use super::{
    CONTROL_HEIGHT, FieldOptions, gate, outlined_field, outlined_field_opts, pill_row, text_button,
};
use eframe::egui::{Sense, accesskit::Role};
use egui_kittest::kittest::{NodeT, Queryable};

#[test]
fn an_inner_enabled_gate_does_not_reenable_an_outer_disabled_one() {
    let harness = themed(400.0, (), |ui, _: &mut ()| {
        ui.horizontal(|ui| {
            gate(ui, false, |ui| {
                gate(ui, true, |ui| text_button(ui, "Inner"));
                text_button(ui, "Outer")
            });
            text_button(ui, "After");
        });
    });
    assert!(harness.get_by_label("Inner").accesskit_node().is_disabled());
    assert!(harness.get_by_label("Outer").accesskit_node().is_disabled());
    // The flag is restored once the outer gate ends.
    assert!(!harness.get_by_label("After").accesskit_node().is_disabled());
}

#[test]
fn a_nested_gate_restores_the_outer_state_for_what_follows() {
    let harness = themed(400.0, (), |ui, _: &mut ()| {
        ui.horizontal(|ui| {
            gate(ui, true, |ui| {
                gate(ui, false, |ui| text_button(ui, "Inner"));
                text_button(ui, "Outer")
            });
        });
    });
    assert!(harness.get_by_label("Inner").accesskit_node().is_disabled());
    assert!(!harness.get_by_label("Outer").accesskit_node().is_disabled());
}

#[test]
fn a_single_line_field_is_exactly_the_control_height() {
    for body_scale in [1.0_f32, 1.1, 1.37] {
        let mut harness = themed(400.0, None, move |ui, height: &mut Option<f32>| {
            ui.style_mut().text_styles.iter_mut().for_each(|(_, font)| {
                font.size *= body_scale;
            });
            let mut text = String::new();
            *height = Some(outlined_field(ui, "Name", &mut text, "").rect.height());
        });
        harness.run();
        let height = harness.state().expect("ran");
        assert!(
            (height - CONTROL_HEIGHT).abs() < 0.01,
            "scale {body_scale}: field is {height} tall, not {CONTROL_HEIGHT}"
        );
    }
}

#[test]
fn the_character_limit_stops_typing_without_cutting_a_value() {
    let mut harness = themed(400.0, String::new(), |ui, text: &mut String| {
        let options = FieldOptions {
            char_limit: Some(5),
            ..FieldOptions::default()
        };
        outlined_field_opts(ui, "Name", text, "", options);
    });
    harness
        .get_by_role_and_label(Role::TextInput, "Name")
        .click();
    harness.run();
    harness
        .get_by_role_and_label(Role::TextInput, "Name")
        .type_text("abcdefghij");
    harness.run();
    assert_eq!(harness.state(), "abcde");
}

#[test]
fn without_a_character_limit_the_field_is_unbounded() {
    let mut harness = themed(400.0, String::new(), |ui, text: &mut String| {
        outlined_field_opts(ui, "Name", text, "", FieldOptions::default());
    });
    harness
        .get_by_role_and_label(Role::TextInput, "Name")
        .click();
    harness.run();
    harness
        .get_by_role_and_label(Role::TextInput, "Name")
        .type_text("abcdefghij");
    harness.run();
    assert_eq!(harness.state(), "abcdefghij");
}

#[test]
fn a_disclosure_row_is_named_by_its_text_and_reports_whether_it_is_open() {
    for open in [true, false] {
        let harness = themed(400.0, (), move |ui, _: &mut ()| {
            pill_row(ui, false, Some(open), "shop", Sense::click());
        });
        let row = harness.get_by_label("shop");
        assert_eq!(row.accesskit_node().data().is_expanded(), Some(open));
        let chevron = crate::theme::icons::chevron(open);
        assert_eq!(
            harness
                .query_all_by_label(&format!("{chevron} shop"))
                .count(),
            0,
            "the private-use chevron must not be part of the name"
        );
    }
}
