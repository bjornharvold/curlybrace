//! The folder header of the connection manager: the disclosure chevron is the theme's icon and
//! sits on the middle of the folder-name row (egui's default triangle sat at the top of it).
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_ui::I18n;
use curlybrace_ui::theme::{self, icons};
use curlybrace_ui::views::connections::{ConnectionsState, show_connections};
use eframe::egui::{self, Shape};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

fn state() -> ConnectionsState {
    let mut in_folder = ConnectionConfig::new("Billing", "mongodb://billing.example.com");
    in_folder.folder = Some("Work".into());
    ConnectionsState::new(vec![in_folder])
}

fn harness() -> Harness<'static, ConnectionsState> {
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 600.0))
        .build_ui_state(
            move |ui, state: &mut ConnectionsState| {
                if !installed {
                    theme::install(ui.ctx());
                    ui.ctx().set_theme(egui::Theme::Light);
                    installed = true;
                }
                show_connections(ui, state, &I18n::english());
            },
            state(),
        );
    harness.run_steps(6);
    harness
}

/// The vertical centres of every text shape drawing exactly `glyph`.
fn glyph_centres(harness: &Harness<'_, ConnectionsState>, glyph: char) -> Vec<f32> {
    fn collect(shape: &Shape, glyph: char, out: &mut Vec<f32>) {
        match shape {
            Shape::Text(t) if t.galley.text() == glyph.to_string() => {
                out.push(t.pos.y + t.galley.rect.center().y);
            }
            Shape::Vec(inner) => inner.iter().for_each(|s| collect(s, glyph, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in &harness.output().shapes {
        collect(&clipped.shape, glyph, &mut out);
    }
    out
}

#[test]
fn the_folder_chevron_is_centred_on_the_folder_name() {
    let harness = harness();
    let name_y = harness.get_by_label("Work").rect().center().y;
    let chevrons = glyph_centres(&harness, icons::chevron(true));
    assert_eq!(chevrons.len(), 1, "one open-folder chevron: {chevrons:?}");
    assert!(
        (chevrons[0] - name_y).abs() <= 2.0,
        "chevron at {} but the name at {name_y}",
        chevrons[0]
    );
}

#[test]
fn clicking_the_folder_header_collapses_and_expands_it() {
    let i18n = I18n::english();
    let mut harness = harness();
    harness.get_by_label("Billing");
    harness
        .get_by_label(&i18n.t("connections.folder_collapse"))
        .click();
    harness.run_steps(12);
    assert!(
        harness.query_by_label("Billing").is_none(),
        "the folder closed"
    );
    assert_eq!(glyph_centres(&harness, icons::chevron(false)).len(), 1);
    harness
        .get_by_label(&i18n.t("connections.folder_expand"))
        .click();
    harness.run_steps(12);
    harness.get_by_label("Billing");
}
