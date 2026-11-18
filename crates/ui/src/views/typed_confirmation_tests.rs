use super::*;
use egui_kittest::Harness;

/// What the box reported in the last frame, and whether the label above it is drawn.
#[derive(Default)]
struct Seen {
    label_above: bool,
    id: Option<egui::Id>,
    text: String,
}

fn harness() -> Harness<'static, Seen> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(500.0, 300.0))
        .build_ui_state(
            |ui, seen: &mut Seen| {
                if seen.label_above {
                    ui.label("A notice that appears above the box");
                }
                let prompt = TypedConfirmation {
                    instruction: "Type this text exactly:",
                    required: "people",
                    field_label: "Collection name",
                    accessible_name: "Type the collection name people to confirm",
                };
                let mut text = std::mem::take(&mut seen.text);
                seen.id = Some(show_typed_confirmation(ui, &prompt, &mut text).id);
                seen.text = text;
            },
            Seen::default(),
        );
    harness.run();
    harness
}

#[test]
fn the_box_keeps_its_id_when_a_widget_above_it_appears_and_disappears() {
    let mut harness = harness();
    let before = harness.state().id;
    assert!(before.is_some());
    for shown in [true, false, true] {
        harness.state_mut().label_above = shown;
        harness.run();
        assert_eq!(harness.state().id, before, "label above shown: {shown}");
    }
}

#[test]
fn two_boxes_with_one_name_in_different_containers_get_different_ids() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(500.0, 400.0))
        .build_ui_state(
            |ui, ids: &mut Vec<egui::Id>| {
                ids.clear();
                for container in ["first", "second"] {
                    ui.push_id(container, |ui| {
                        let prompt = TypedConfirmation {
                            instruction: "Type this text exactly:",
                            required: "people",
                            field_label: "Collection name",
                            accessible_name: "Type the collection name people to confirm",
                        };
                        let mut text = String::new();
                        ids.push(show_typed_confirmation(ui, &prompt, &mut text).id);
                    });
                }
            },
            Vec::new(),
        );
    harness.run();
    let ids = harness.state();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
}
