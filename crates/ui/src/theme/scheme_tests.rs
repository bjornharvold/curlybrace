use super::contrast::ratio;
use super::{Mode, Scheme, Tone, scheme};
use eframe::egui::Color32;

const MODES: [Mode; 2] = [Mode::Light, Mode::Dark];

fn hex(h: u32) -> Color32 {
    Color32::from_rgb((h >> 16) as u8, (h >> 8) as u8, h as u8)
}

#[test]
fn light_and_dark_use_the_mockup_primaries() {
    assert_eq!(scheme(Mode::Light).primary, hex(0x2C6B30));
    assert_eq!(scheme(Mode::Dark).primary, hex(0x8BD88A));
    assert_eq!(scheme(Mode::Light).mode(), Mode::Light);
    assert_eq!(scheme(Mode::Dark).mode(), Mode::Dark);
}

#[test]
fn ratio_of_black_on_white_is_21() {
    let r = ratio(Color32::BLACK, Color32::WHITE);
    assert!((r - 21.0).abs() < 0.01, "got {r}");
    assert!((ratio(Color32::WHITE, Color32::BLACK) - r).abs() < 1e-4);
    assert!((ratio(Color32::WHITE, Color32::WHITE) - 1.0).abs() < 1e-4);
}

fn surfaces(s: &Scheme) -> [(&'static str, Color32); 5] {
    [
        ("background", s.background),
        ("surface", s.surface),
        ("surface_container_low", s.surface_container_low),
        ("surface_container", s.surface_container),
        ("surface_container_high", s.surface_container_high),
    ]
}

fn check(mode: Mode, name: &str, fg: Color32, bg: Color32, min: f32) {
    let r = ratio(fg, bg);
    assert!(r >= min, "{mode:?} {name}: {r:.2} < {min}");
}

#[test]
fn every_text_pair_meets_aa_in_both_modes() {
    for mode in MODES {
        let s = scheme(mode);
        for (n, bg) in surfaces(s) {
            check(mode, &format!("on_surface/{n}"), s.on_surface, bg, 4.5);
            check(
                mode,
                &format!("on_variant/{n}"),
                s.on_surface_variant,
                bg,
                4.5,
            );
        }
        check(mode, "on_primary", s.on_primary, s.primary, 4.5);
        check(
            mode,
            "on_primary_container",
            s.on_primary_container,
            s.primary_container,
            4.5,
        );
        check(
            mode,
            "on_secondary_container",
            s.on_secondary_container,
            s.secondary_container,
            4.5,
        );
        check(mode, "on_error", s.on_error, s.error, 4.5);
        check(
            mode,
            "on_error_container",
            s.on_error_container,
            s.error_container,
            4.5,
        );
        for tone in [Tone::Good, Tone::Warning, Tone::Bad, Tone::Neutral] {
            let t = s.tone(tone);
            check(
                mode,
                &format!("{tone:?} on_container"),
                t.on_container,
                t.container,
                4.5,
            );
        }
        check(mode, "outline/background", s.outline, s.background, 3.0);
        check(mode, "primary/background", s.primary, s.background, 3.0);
    }
}

#[test]
fn success_and_warning_pairs_meet_aa() {
    for mode in MODES {
        let s = scheme(mode);
        check(mode, "on_success", s.on_success, s.success, 4.5);
        check(mode, "on_warning", s.on_warning, s.warning, 4.5);
        check(
            mode,
            "on_success_container",
            s.on_success_container,
            s.success_container,
            4.5,
        );
        check(
            mode,
            "on_warning_container",
            s.on_warning_container,
            s.warning_container,
            4.5,
        );
    }
}

#[test]
fn mockup_given_values_are_verbatim() {
    let l = scheme(Mode::Light);
    assert_eq!(l.on_primary, hex(0xFFFFFF));
    assert_eq!(l.primary_container, hex(0xB0F2AC));
    assert_eq!(l.secondary_container, hex(0xD4E8CF));
    assert_eq!(l.background, hex(0xFBFDF6));
    assert_eq!(l.surface_container_low, hex(0xF0F5EA));
    assert_eq!(l.surface_container_high, hex(0xE4E9DE));
    assert_eq!(l.outline, hex(0x72796F));
    assert_eq!(l.outline_variant, hex(0xC2C9BD));
    let d = scheme(Mode::Dark);
    assert_eq!(d.on_primary, hex(0x00390F));
    assert_eq!(d.primary_container, hex(0x1E5127));
    assert_eq!(d.secondary_container, hex(0x3A4B39));
    assert_eq!(d.background, hex(0x10140F));
    assert_eq!(d.surface_container_low, hex(0x181D17));
    assert_eq!(d.surface_container_high, hex(0x262B24));
    assert_eq!(d.outline, hex(0x8B9388));
    assert_eq!(d.outline_variant, hex(0x414940));
}

#[test]
fn shape_and_spacing_scales_follow_material() {
    use super::shape::*;
    assert_eq!(
        [EXTRA_SMALL, SMALL, MEDIUM, LARGE, EXTRA_LARGE, FULL],
        [4.0, 8.0, 12.0, 16.0, 28.0, 999.0]
    );
    assert_eq!(
        [SPACE_1, SPACE_2, SPACE_3, SPACE_4, SPACE_5, SPACE_6],
        [4.0, 8.0, 12.0, 16.0, 20.0, 24.0]
    );
}
