//! A toolbar that wraps keeps every button at its natural width: a disabled button must not be
//! cut to the room left on the line (which `add_enabled_ui` did) but move to the next line.
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_ui::I18n;
use curlybrace_ui::theme;
use curlybrace_ui::views::connections::{ConnectionsState, show_connections};
use eframe::egui::{self, accesskit::Role};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

const KEYS: [&str; 7] = [
    "connections.new",
    "connections.edit",
    "connections.duplicate",
    "common.delete",
    "connections.import",
    "connections.export",
    "connections.move_to_folder",
];

fn manager(width: f32, selected: bool) -> Harness<'static, ConnectionsState> {
    let config = ConnectionConfig::new("Alpha", "mongodb://alpha.example.com");
    let mut state = ConnectionsState::new(vec![config.clone()]);
    state.selected = selected.then_some(config.id);
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(width, 600.0))
        .build_ui_state(
            move |ui, state: &mut ConnectionsState| {
                if !installed {
                    theme::install(ui.ctx());
                    ui.ctx().set_theme(egui::Theme::Light);
                    installed = true;
                }
                show_connections(ui, state, &I18n::english());
            },
            state,
        );
    harness.run_steps(6);
    harness
}

#[test]
fn the_connections_toolbar_keeps_its_buttons_whole_when_it_wraps() {
    for selected in [false, true] {
        let wide = manager(1600.0, selected);
        let narrow = manager(400.0, selected);
        let i18n = I18n::english();
        let mut tops = Vec::new();
        for key in KEYS {
            let label = i18n.t(key);
            let natural = wide.get_by_role_and_label(Role::Button, &label).rect();
            let cut = narrow.get_by_role_and_label(Role::Button, &label).rect();
            assert!(
                (natural.width() - cut.width()).abs() < 0.5,
                "{label} (selected: {selected}) was cut from {} to {}",
                natural.width(),
                cut.width()
            );
            assert!(cut.right() <= 400.0, "{label} runs past the row: {cut:?}");
            tops.push(cut.top());
        }
        let first = tops[0];
        assert!(
            tops.iter().any(|t| *t > first + 1.0),
            "the toolbar did not wrap"
        );
    }
}
