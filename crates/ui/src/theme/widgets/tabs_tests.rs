use super::tabs;
use crate::theme::{Mode, install, scheme};
use eframe::egui::{self, Rect, Shape, accesskit::Role, accesskit::Toggled};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

#[derive(Default)]
struct State {
    selected: u8,
    reported: Option<u8>,
}

fn harness() -> Harness<'static, State> {
    let items = vec![(0u8, "Documents".to_owned()), (1u8, "Indexes".to_owned())];
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(400.0, 100.0))
        .build_ui_state(
            move |ui, state: &mut State| {
                if !installed {
                    install(ui.ctx());
                    ui.ctx().set_theme(egui::Theme::Light);
                    installed = true;
                }
                if let Some(picked) = tabs(ui, &items, state.selected) {
                    state.selected = picked;
                    state.reported = Some(picked);
                }
            },
            State::default(),
        );
    harness.run();
    harness
}

#[test]
fn tabs_report_the_clicked_tab_and_their_selection() {
    let mut harness = harness();
    let selection = |h: &Harness<'_, State>, label: &str| {
        h.get_by_role_and_label(Role::Button, label)
            .accesskit_node()
            .toggled()
    };
    assert_eq!(selection(&harness, "Documents"), Some(Toggled::True));
    assert_eq!(selection(&harness, "Indexes"), Some(Toggled::False));
    harness.get_by_label("Documents").click();
    harness.run();
    assert_eq!(
        harness.state().reported,
        None,
        "the selected tab is no change"
    );
    harness.get_by_label("Indexes").click();
    harness.run();
    assert_eq!(harness.state().reported, Some(1));
    assert_eq!(selection(&harness, "Indexes"), Some(Toggled::True));
}

#[test]
fn the_selected_tab_has_a_three_dp_primary_indicator() {
    let harness = harness();
    let primary = scheme(Mode::Light).primary;
    let tab = harness.get_by_label("Documents").rect();
    let bars: Vec<Rect> = harness
        .output()
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            Shape::Rect(r) if r.fill == primary => Some(r.rect),
            _ => None,
        })
        .collect();
    assert_eq!(bars.len(), 1, "one indicator: {bars:?}");
    assert!((bars[0].height() - 3.0).abs() < 0.01);
    assert!((bars[0].bottom() - tab.bottom()).abs() < 0.01);
    assert!(tab.x_range().contains(bars[0].center().x));
}
