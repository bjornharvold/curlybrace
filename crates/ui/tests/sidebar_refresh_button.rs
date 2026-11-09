//! The sidebar's Refresh is the theme's compact text button (40 dp, pill), not egui's small
//! button, in both sidebar layouts; its name and action are unchanged.
use curlybrace_ui::I18n;
use curlybrace_ui::theme;
use curlybrace_ui::viewmodel::SidebarState;
use curlybrace_ui::views::sidebar::{SidebarAction, show_sidebar, show_sidebar_body};
use eframe::egui::{self, accesskit::Role};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

type Seen = Vec<SidebarAction>;

fn harness(body: bool) -> Harness<'static, Seen> {
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(300.0, 300.0))
        .build_ui_state(
            move |ui, seen: &mut Seen| {
                if !installed {
                    theme::install(ui.ctx());
                    ui.ctx().set_theme(egui::Theme::Light);
                    installed = true;
                }
                let state = SidebarState::default();
                let i18n = I18n::english();
                let action = match body {
                    true => show_sidebar_body(ui, &state, None, &i18n),
                    false => show_sidebar(ui, &state, &i18n),
                };
                seen.extend(action);
            },
            Vec::new(),
        );
    harness.run_steps(4);
    harness
}

#[test]
fn refresh_is_a_theme_button_in_both_layouts_and_still_refreshes() {
    let label = I18n::english().t("sidebar.refresh");
    for body in [false, true] {
        let mut harness = harness(body);
        let refresh = harness.get_by_role_and_label(Role::Button, &label);
        assert_eq!(refresh.rect().height(), 40.0, "body: {body}");
        refresh.click();
        harness.run_steps(3);
        assert_eq!(
            harness.state(),
            &vec![SidebarAction::Refresh],
            "body: {body}"
        );
    }
}
