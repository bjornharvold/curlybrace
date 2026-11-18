use super::*;

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn line(name: &str, hint: &str) -> String {
    let (name, hint) = (t(name), t(hint));
    I18n::english().t_with(
        "workspace.switcher_hint_line",
        &[("name", &name), ("hint", &hint)],
    )
}

#[test]
fn the_hint_names_each_entry_through_the_translation() {
    let text = hints(&I18n::english(), true);
    let expected = [
        line("workspace.browse", "workspace.browse_hint"),
        line("workspace.compare", "workspace.compare_hint"),
        line("workspace.shell", "workspace.shell_hint"),
    ]
    .join("\n");
    assert_eq!(text, expected);
}

#[test]
fn while_not_connected_the_hint_says_why_the_shell_is_off() {
    let text = hints(&I18n::english(), false);
    let shell = line("workspace.shell", "workspace.shell_hint_not_connected");
    assert!(text.ends_with(&shell), "{text}");
}

/// (the Compare segment's width, the label's natural width in the segment font plus its padding)
/// with the switcher in a side panel of the default sidebar width.
fn compare_segment_and_label() -> (f32, f32) {
    use crate::theme::widgets::{SEGMENT_PADDING, medium_font};
    use crate::views::sidebar_roots::SIDEBAR_DEFAULT_WIDTH;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(800.0, 300.0))
        .build_state(
            move |ctx, label_width: &mut f32| {
                if !installed {
                    crate::theme::install(ctx);
                    ctx.set_theme(egui::Theme::Light);
                    installed = true;
                }
                egui::SidePanel::left("sidebar")
                    .resizable(true)
                    .default_width(SIDEBAR_DEFAULT_WIDTH)
                    .show(ctx, |ui| {
                        let size = egui::TextStyle::Button.resolve(ui.style()).size;
                        let font = medium_font(ui, size);
                        *label_width = ui
                            .painter()
                            .layout_no_wrap(t("workspace.compare"), font, egui::Color32::BLACK)
                            .size()
                            .x
                            + 2.0 * SEGMENT_PADDING;
                        show_workspace_switcher(ui, Workspace::Browse, true, &I18n::english());
                    });
            },
            0.0,
        );
    harness.run();
    let segment = harness
        .get_by_role_and_label(egui::accesskit::Role::RadioButton, &t("workspace.compare"))
        .rect()
        .width();
    (segment, *harness.state())
}

/// A guard of existing behaviour (it passes before and after this change): the sidebar's default
/// width leaves Compare its natural width, so its label is never elided.
#[test]
fn compare_is_laid_out_in_full_at_the_default_sidebar_width() {
    let (segment, natural) = compare_segment_and_label();
    assert!(
        segment >= natural - 0.5,
        "the Compare segment is {segment} wide, its label needs {natural}"
    );
}
