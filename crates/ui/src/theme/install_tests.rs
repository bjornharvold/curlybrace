use super::shape;
use super::typography;
use super::{Mode, StateLayer, current, install, scheme, state_layer, visuals_for};
use eframe::egui::{self, Color32, TextStyle};
use egui_kittest::Harness;

#[test]
fn visuals_use_the_scheme_surfaces() {
    for mode in [Mode::Light, Mode::Dark] {
        let s = scheme(mode);
        let v = visuals_for(s);
        assert_eq!(v.panel_fill, s.surface_container_low);
        assert_eq!(v.selection.bg_fill, s.secondary_container);
        assert_eq!(v.window_fill, s.surface_container_high);
        assert_eq!(v.dark_mode, mode == Mode::Dark);
    }
}

#[test]
fn state_layer_blends_toward_the_on_color() {
    let base = Color32::from_rgb(200, 200, 200);
    let on = Color32::from_rgb(0, 0, 0);
    // 8% of black over 200 -> 200 * 0.92 = 184
    assert_eq!(
        state_layer(base, on, StateLayer::Hover),
        Color32::from_rgb(184, 184, 184)
    );
    // 10% -> 180
    assert_eq!(
        state_layer(base, on, StateLayer::Pressed),
        Color32::from_rgb(180, 180, 180)
    );
    assert_eq!(
        state_layer(base, on, StateLayer::Focus),
        state_layer(base, on, StateLayer::Pressed)
    );
}

#[test]
fn radius_clamps_the_full_shape_to_a_pill() {
    assert_eq!(
        shape::radius(shape::FULL),
        egui::CornerRadius::same(u8::MAX)
    );
    assert_eq!(shape::radius(shape::SMALL), egui::CornerRadius::same(8));
    assert_eq!(shape::radius(-3.0), egui::CornerRadius::same(0));
}

#[test]
fn switching_the_theme_changes_the_panel_color() {
    let mut harness = Harness::builder()
        .with_theme(egui::Theme::Light)
        .build_ui(|ui| {
            ui.label("x");
        });
    install(&harness.ctx);
    harness.ctx.set_theme(egui::Theme::Light);
    harness.run();
    let light = harness.ctx.style().visuals.panel_fill;
    assert_eq!(light, scheme(Mode::Light).surface_container_low);
    assert_eq!(current(&harness.ctx).mode(), Mode::Light);
    harness.ctx.set_theme(egui::Theme::Dark);
    harness.run();
    let dark = harness.ctx.style().visuals.panel_fill;
    assert_eq!(dark, scheme(Mode::Dark).surface_container_low);
    assert_ne!(light, dark);
    assert_eq!(current(&harness.ctx).mode(), Mode::Dark);
}

#[test]
fn text_styles_lay_out_at_two_pixels_per_point() {
    let styles = typography::text_styles();
    assert!(styles.contains_key(&TextStyle::Body));
    assert!(styles.contains_key(&typography::title_medium()));
    assert!(styles.contains_key(&typography::title_large()));
    for style in styles.keys() {
        let mut harness = Harness::builder()
            .with_pixels_per_point(2.0)
            .with_size(egui::vec2(400.0, 200.0))
            .build_ui(|ui| {
                ui.label("Connections");
            });
        install(&harness.ctx);
        harness.run();
        let font_id = harness.ctx.style().text_styles[style].clone();
        let rows = harness.ctx.fonts(|f| {
            f.layout("Connections".into(), font_id, Color32::BLACK, 300.0)
                .rows
                .len()
        });
        assert_eq!(rows, 1, "{style:?} wrapped a one-line label");
    }
}
