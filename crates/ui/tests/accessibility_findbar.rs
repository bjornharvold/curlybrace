//! The automatic-verdict checkbox in the accesskit tree, rendered with the REAL English locale:
//! it has its accessible name, shows its state, reports a toggle and never shows a missing-key
//! marker. Fast, offline.
use curlybrace_ui::I18n;
use curlybrace_ui::views::findbar::{FindAction, show_automatic_verdict};
use eframe::egui;
use eframe::egui::accesskit::{Role, Toggled};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

const LABEL: &str = "Check index use automatically";

struct Frame {
    automatic: bool,
    seen: Vec<FindAction>,
}

fn open(automatic: bool) -> Harness<'static, Frame> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 200.0))
        .build_ui_state(
            |ui, frame: &mut Frame| {
                let action = show_automatic_verdict(ui, frame.automatic, &I18n::english());
                frame.seen.extend(action);
            },
            Frame {
                automatic,
                seen: Vec::new(),
            },
        );
    harness.run_steps(4);
    harness
}

#[test]
fn the_checkbox_has_its_label_and_state_and_no_missing_key_marker() {
    for automatic in [true, false] {
        let harness = open(automatic);
        let checkbox = harness.get_by_role_and_label(Role::CheckBox, LABEL);
        let expected = if automatic {
            Toggled::True
        } else {
            Toggled::False
        };
        assert_eq!(checkbox.accesskit_node().toggled(), Some(expected));
        let mut labels = Vec::new();
        for node in harness.root().children_recursive() {
            labels.extend(node.accesskit_node().label());
        }
        assert!(labels.iter().all(|l| !l.contains('\u{27e6}')), "{labels:?}");
    }
}

#[test]
fn clicking_reports_the_opposite_setting() {
    let mut harness = open(true);
    harness.get_by_role_and_label(Role::CheckBox, LABEL).click();
    harness.run_steps(4);
    assert_eq!(
        harness.state().seen,
        vec![FindAction::SetAutomaticVerdict(false)]
    );
}
