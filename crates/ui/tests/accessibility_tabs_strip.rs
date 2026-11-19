//! The tab strip drawn on its own (#159, review fixes): the New tab button says why it is off.
mod support;

use curlybrace_ui::I18n;
use curlybrace_ui::app::tabs::TabId;
use curlybrace_ui::theme;
use curlybrace_ui::viewmodel::ConnectionBadge;
use curlybrace_ui::viewmodel::tab_title::TabLabel;
use curlybrace_ui::views::tab_strip::{StripView, show_tab_strip};
use eframe::egui::{self, accesskit::Role};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

fn label(title: &str) -> (TabId, TabLabel) {
    (
        TabId(1),
        TabLabel {
            title: title.into(),
            tooltip: title.into(),
            accessible: format!("{title}, tab 1 of 1"),
            menu_name: title.into(),
            badge: ConnectionBadge::default(),
            dormant: false,
            unsaved: false,
        },
    )
}

fn strip(can_open: bool) -> Harness<'static, ()> {
    let i18n = I18n::english();
    let labels = vec![label("users")];
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 80.0))
        .build_ui(move |ui| {
            if !installed {
                theme::install(ui.ctx());
                installed = true;
            }
            let view = StripView {
                labels: &labels,
                active: Some(TabId(1)),
                limit_reached: false,
                can_reopen: false,
                can_open,
                split: Default::default(),
            };
            show_tab_strip(ui, &view, &i18n);
        });
    harness.run_steps(4);
    harness
}

#[test]
fn a_focused_text_field_scrolled_out_of_view_still_counts_as_typing() {
    use curlybrace_ui::views::tab_shortcuts::text_has_focus;
    let id = egui::Id::new("scrolled_out_field");
    let mut text = String::new();
    let mut frames = 0;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(300.0, 100.0))
        .build_ui(move |ui| {
            // Nothing of it is visible: egui reports no IME rectangle for such a field.
            ui.scope(|ui| {
                ui.set_clip_rect(egui::Rect::NOTHING);
                let field = ui.add(egui::TextEdit::singleline(&mut text).id(id));
                if frames == 0 {
                    field.request_focus();
                }
            });
            frames += 1;
        });
    harness.run_steps(4);
    let ctx = harness.ctx.clone();
    assert!(
        ctx.memory(|m| m.has_focus(id)),
        "the field has the keyboard"
    );
    assert!(
        text_has_focus(&ctx),
        "typing is typing wherever the field is scrolled to"
    );
    // A focused button is not a text field.
    let button = egui::Id::new("a_button");
    ctx.memory_mut(|m| m.request_focus(button));
    assert!(!text_has_focus(&ctx));
}

#[test]
fn the_new_tab_button_is_off_and_says_why_without_a_connection() {
    let i18n = I18n::english();
    let off = strip(false);
    let disabled = i18n.t("tabs.new_tab_disabled");
    let plain = i18n.t("tabs.new_tab");
    let button = off.get_by_role_and_label(Role::Button, &disabled);
    assert!(
        button.accesskit_node().is_disabled(),
        "it cannot be pressed"
    );
    assert!(off.query_by_role_and_label(Role::Button, &plain).is_none());
    let on = strip(true);
    let button = on.get_by_role_and_label(Role::Button, &plain);
    assert!(!button.accesskit_node().is_disabled());
}
