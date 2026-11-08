use super::{card, chip, filled_button, outlined_field, pill, segmented, with_alpha};
use crate::theme::{Mode, Tone, install, scheme};
use eframe::egui::{self, Rect, Shape, accesskit::Role};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

/// A harness of `width` x 200 with the theme installed and the light mode active.
fn themed<S>(
    width: f32,
    state: S,
    mut app: impl FnMut(&mut egui::Ui, &mut S) + 'static,
) -> Harness<'static, S> {
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(width, 200.0))
        .build_ui_state(
            move |ui, state| {
                if !installed {
                    install(ui.ctx());
                    ui.ctx().set_theme(egui::Theme::Light);
                    installed = true;
                }
                app(ui, state);
            },
            state,
        );
    harness.run();
    harness
}

#[derive(Default)]
struct Pick {
    selected: u8,
    reported: Option<u8>,
}

fn items(labels: &[&str]) -> Vec<(u8, String)> {
    (0u8..)
        .zip(labels.iter().map(|l| (*l).to_owned()))
        .collect()
}

#[test]
fn segmented_reports_the_clicked_item() {
    let options = items(&["One", "Two", "Three"]);
    let mut harness = themed(400.0, Pick::default(), move |ui, state: &mut Pick| {
        if let Some(picked) = segmented(ui, &options, state.selected) {
            state.selected = picked;
            state.reported = Some(picked);
        }
    });
    harness.get_by_label("Three").click();
    harness.run();
    assert_eq!(harness.state().reported, Some(2));
    assert_eq!(harness.state().selected, 2);
    // Clicking the selected item again is not a change.
    harness.state_mut().reported = None;
    harness.get_by_label("Three").click();
    harness.run();
    assert_eq!(harness.state().reported, None);
}

#[derive(Default)]
struct Bounds {
    available: Option<Rect>,
    widget: Option<Rect>,
}

const LONG: &str = "An extraordinarily long label that would never fit in a narrow sidebar column";

#[test]
fn segmented_with_long_labels_stays_inside_its_width() {
    let options = items(&[LONG, "Short", LONG]);
    let mut harness = themed(220.0, Bounds::default(), move |ui, state: &mut Bounds| {
        state.available = Some(ui.available_rect_before_wrap());
        segmented(ui, &options, 0u8);
    });
    harness.run();
    let available = harness.state().available.expect("ran");
    let rects: Vec<Rect> = harness
        .get_all_by_role(Role::RadioButton)
        .map(|n| n.rect())
        .collect();
    assert_eq!(rects.len(), 3);
    for (i, rect) in rects.iter().enumerate() {
        assert!(
            rect.min.x >= available.min.x - 0.5 && rect.max.x <= available.max.x + 0.5,
            "segment {i} {rect:?} leaves {available:?}"
        );
        if let Some(next) = rects.get(i + 1) {
            assert!(rect.max.x <= next.min.x + 0.5, "segments overlap");
        }
    }
    // The full text stays available to screen readers even though it is drawn elided.
    assert_eq!(harness.get_all_by_label(LONG).count(), 2);
}

#[test]
fn chip_with_long_text_truncates() {
    let mut harness = themed(160.0, Bounds::default(), |ui, state: &mut Bounds| {
        state.available = Some(ui.available_rect_before_wrap());
        state.widget = Some(chip(ui, Tone::Warning, LONG).rect);
    });
    harness.run();
    let available = harness.state().available.expect("ran");
    let chip_rect = harness.state().widget.expect("ran");
    assert!(
        chip_rect.max.x <= available.max.x + 0.5,
        "chip {chip_rect:?} overflows {available:?}"
    );
    assert!(chip_rect.height() < 40.0, "chip wrapped: {chip_rect:?}");
    harness.get_by_role_and_label(Role::Label, LONG);
}

#[test]
fn chip_with_short_text_hugs_it() {
    let mut harness = themed(400.0, Bounds::default(), |ui, state: &mut Bounds| {
        state.available = Some(ui.available_rect_before_wrap());
        state.widget = Some(chip(ui, Tone::Good, "OK").rect);
    });
    harness.run();
    let available = harness.state().available.expect("ran");
    let chip_rect = harness.state().widget.expect("ran");
    assert!(chip_rect.width() < available.width() / 4.0);
}

#[test]
fn outlined_field_edits_the_string() {
    let mut harness = themed(400.0, String::new(), |ui, text: &mut String| {
        outlined_field(ui, "Name", text, "e.g. Production");
    });
    harness.get_by_label("Name").click();
    harness.run();
    harness.get_by_label("Name").type_text("abc");
    harness.run();
    assert_eq!(harness.state(), "abc");
    assert_eq!(harness.get_by_label("Name").value().as_deref(), Some("abc"));
}

/// Every filled rectangle painted in the last frame, with its fill.
fn filled_rects(harness: &Harness<'_, Bounds>) -> Vec<(Rect, egui::Color32)> {
    harness
        .output()
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            Shape::Rect(r) => Some((r.rect, r.fill)),
            _ => None,
        })
        .collect()
}

#[test]
fn pill_selected_uses_the_secondary_container() {
    let container = scheme(Mode::Light).secondary_container;
    for selected in [true, false] {
        let mut harness = themed(300.0, Bounds::default(), move |ui, state: &mut Bounds| {
            state.widget = Some(pill(ui, selected, "customers").rect);
        });
        harness.run();
        let rect = harness.state().widget.expect("ran");
        let filled = filled_rects(&harness)
            .into_iter()
            .any(|(r, fill)| fill == container && r == rect);
        assert_eq!(filled, selected, "selected={selected}");
    }
}

#[test]
fn card_uses_the_low_surface_and_the_medium_radius() {
    let s = scheme(Mode::Light);
    let mut harness = themed(300.0, Bounds::default(), |ui, state: &mut Bounds| {
        state.widget = Some(card(ui, |ui| ui.label("inside")).response.rect);
    });
    harness.run();
    let rect = harness.state().widget.expect("ran");
    let found = harness.output().shapes.iter().any(|c| match &c.shape {
        Shape::Rect(r) => {
            r.rect == rect
                && r.fill == s.surface_container_low
                && r.stroke.color == s.outline_variant
                && r.corner_radius == crate::theme::shape::radius(crate::theme::shape::MEDIUM)
        }
        _ => false,
    });
    assert!(found, "no card frame at {rect:?}");
}

#[test]
fn disabled_filled_button_uses_material_opacities() {
    let s = scheme(Mode::Light);
    let mut harness = themed(300.0, Bounds::default(), |ui, state: &mut Bounds| {
        ui.add_enabled_ui(false, |ui| {
            state.widget = Some(filled_button(ui, "Save").rect);
        });
    });
    harness.run();
    let rect = harness.state().widget.expect("ran");
    let container = with_alpha(s.on_surface, 12);
    assert!(
        filled_rects(&harness)
            .into_iter()
            .any(|(r, fill)| r == rect && fill == container),
        "disabled container is not 12% on_surface"
    );
}

#[test]
fn with_alpha_maps_percent_to_alpha() {
    let c = egui::Color32::from_rgb(10, 20, 30);
    assert_eq!(with_alpha(c, 100).a(), 255);
    assert_eq!(with_alpha(c, 0).a(), 0);
    assert_eq!(with_alpha(c, 38).a(), 97);
}

#[test]
fn outlined_field_error_paints_the_error_color() {
    let s = scheme(Mode::Light);
    let mut harness = themed(300.0, String::new(), |ui, text: &mut String| {
        super::outlined_field_with(ui, "TTL", text, "seconds", Some("Must be a number"));
    });
    harness.run();
    let stroked = harness.output().shapes.iter().any(|c| match &c.shape {
        Shape::Rect(r) => r.stroke.color == s.error,
        _ => false,
    });
    assert!(stroked, "no border in the error color");
}

#[test]
fn error_meets_contrast_on_the_field_surfaces() {
    use crate::theme::contrast::ratio;
    for mode in [Mode::Light, Mode::Dark] {
        let s = scheme(mode);
        for surface in [
            s.background,
            s.surface,
            s.surface_container_low,
            s.surface_container,
            s.surface_container_high,
        ] {
            // The supporting text is small text: AA 4.5; the border is a boundary: 3:1.
            let r = ratio(s.error, surface);
            assert!(r >= 4.5, "{mode:?}: error on {surface:?} is {r}");
        }
    }
}

/// Every focus ring painted (2 dp `primary` stroke), with its painted extent and clip rect.
fn rings(output: &egui::FullOutput, primary: egui::Color32) -> Vec<(Rect, Rect)> {
    output
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            Shape::Rect(r) if r.stroke.color == primary && r.stroke.width >= 2.0 => {
                let extent = match r.stroke_kind {
                    egui::StrokeKind::Inside => r.rect,
                    egui::StrokeKind::Middle => r.rect.expand(r.stroke.width / 2.0),
                    egui::StrokeKind::Outside => r.rect.expand(r.stroke.width),
                };
                Some((extent, c.clip_rect))
            }
            _ => None,
        })
        .collect()
}

/// A frameless, narrow side panel (as the sidebar is) with `first` at the very top of a scroll
/// area, so a ring drawn outside the widget would cross the clip rect.
fn side_panel(first: fn(&mut egui::Ui)) -> Harness<'static> {
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(500.0, 300.0))
        .build(move |ctx| {
            if !installed {
                install(ctx);
                ctx.set_theme(egui::Theme::Light);
                installed = true;
            }
            egui::SidePanel::left("side")
                .frame(egui::Frame::NONE)
                .exact_width(180.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, first);
                });
        });
    harness.run();
    harness
}

fn assert_ring_inside_clip(first: fn(&mut egui::Ui), label: &str) {
    let primary = scheme(Mode::Light).primary;
    let mut harness = side_panel(first);
    harness.get_all_by_label(label).next().expect(label).focus();
    harness.run();
    let painted = rings(harness.output(), primary);
    assert!(!painted.is_empty(), "{label}: no focus ring");
    for (extent, clip) in painted {
        assert!(
            clip.expand(0.5).contains_rect(extent),
            "{label}: ring {extent:?} leaves clip {clip:?}"
        );
    }
}

#[test]
fn focus_ring_stays_inside_the_clip_of_full_width_widgets() {
    assert_ring_inside_clip(
        |ui| {
            pill(ui, true, "customers");
        },
        "customers",
    );
    assert_ring_inside_clip(
        |ui| {
            segmented(ui, &items(&[LONG, LONG]), 0u8);
        },
        LONG,
    );
}

#[test]
fn pill_with_unbounded_width_stays_finite() {
    let mut harness = themed(300.0, Bounds::default(), |ui, state: &mut Bounds| {
        // What a horizontal scroll area's content sees: no bound on the width.
        ui.allocate_ui(egui::vec2(f32::INFINITY, 40.0), |ui| {
            state.widget = Some(pill(ui, false, "customers").rect);
        });
    });
    harness.run();
    let rect = harness.state().widget.expect("ran");
    assert!(
        rect.width().is_finite() && rect.width() < 10_000.0,
        "{rect:?}"
    );
}
