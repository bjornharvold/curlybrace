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
    let mut harness = Harness::builder()
        .with_pixels_per_point(2.0)
        .with_size(egui::vec2(400.0, 200.0))
        .build_ui(|ui| {
            ui.label("Connections");
        });
    install(&harness.ctx);
    harness.run();
    let ctx = &harness.ctx;
    assert_eq!(ctx.pixels_per_point(), 2.0);

    let styles = typography::text_styles();
    assert!(styles.contains_key(&typography::title_medium()));
    assert!(styles.contains_key(&typography::title_large()));
    for (style, font) in &styles {
        let physical = font.size * ctx.pixels_per_point();
        assert_eq!(
            physical.fract(),
            0.0,
            "{style:?} is not whole physical pixels"
        );
    }

    let height = |style: TextStyle| {
        let font_id = ctx.style().text_styles[&style].clone();
        ctx.fonts(|f| f.row_height(&font_id)) * ctx.pixels_per_point()
    };
    let scale = [
        TextStyle::Small,
        TextStyle::Body,
        typography::title_medium(),
        typography::title_large(),
        TextStyle::Heading,
    ];
    let heights: Vec<f32> = scale.into_iter().map(height).collect();
    assert!(heights.iter().all(|h| *h > 0.0), "{heights:?}");
    assert!(
        heights.windows(2).all(|w| w[0] < w[1]),
        "heights must grow along the Material scale: {heights:?}"
    );
}

#[test]
fn install_follows_the_system_theme_preference() {
    let harness = Harness::builder().build_ui(|_| {});
    install(&harness.ctx);
    assert_eq!(
        harness.ctx.options(|o| o.theme_preference),
        egui::ThemePreference::System
    );
}

#[test]
fn native_widgets_keep_a_small_corner_radius() {
    for mode in [Mode::Light, Mode::Dark] {
        let v = visuals_for(scheme(mode));
        for w in [&v.widgets.inactive, &v.widgets.hovered, &v.widgets.active] {
            assert!(w.corner_radius.nw <= 4, "radius {:?}", w.corner_radius);
        }
    }
}

/// Every rect the checkbox paints must be square-ish; a radius of half the 14 px box is a circle.
#[test]
fn a_rendered_checkbox_is_not_a_circle_in_both_modes() {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let ctx = egui::Context::default();
        ctx.set_theme(theme);
        install(&ctx);
        let mut checked = false;
        let out = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.checkbox(&mut checked, "x");
            });
        });
        let radii: Vec<u8> = out
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.rect.width() < 20.0 && r.rect.width() > 8.0 => {
                    Some(r.corner_radius.nw)
                }
                _ => None,
            })
            .collect();
        assert!(!radii.is_empty(), "no checkbox box painted in {theme:?}");
        assert!(radii.iter().all(|r| *r <= 4), "{theme:?}: radii {radii:?}");
    }
}
