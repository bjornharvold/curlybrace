use super::fonts::{MEDIUM_FAMILY, SYMBOLS_FAMILY, font_definitions};
use super::icons;
use super::{install, typography};
use eframe::egui::{self, Context, FontFamily, FontId, TextStyle};
use std::collections::BTreeSet;

const BUDGET_BYTES: usize = 1_500_000;

/// Glyphs drawn from code that the theme does not own (connection tag symbols).
const TAG_GLYPHS: &str = "♥▶■♣★♦";

/// A context with the theme installed and one frame run, so fonts exist.
fn ready_context() -> Context {
    let ctx = Context::default();
    install(&ctx);
    let _ = ctx.run(egui::RawInput::default(), |_| {});
    ctx
}

fn non_ascii(text: &str) -> BTreeSet<char> {
    text.chars().filter(|c| !c.is_ascii()).collect()
}

#[test]
fn fonts_stay_within_the_size_budget() {
    let defs = font_definitions();
    let ours = ["roboto-regular", "roboto-medium", "symbols"];
    let total: usize = ours.iter().map(|n| defs.font_data[*n].font.len()).sum();
    assert!(total <= BUDGET_BYTES, "bundled fonts are {total} bytes");
}

#[test]
fn every_glyph_the_ui_draws_resolves() {
    let ctx = ready_context();
    let text = include_str!("../../locales/en.json");
    let mut missing = Vec::new();
    ctx.fonts(|f| {
        let body = FontId::new(14.0, FontFamily::Proportional);
        let medium = FontId::new(14.0, FontFamily::Name(MEDIUM_FAMILY.into()));
        let symbols = FontId::new(24.0, FontFamily::Name(SYMBOLS_FAMILY.into()));
        for c in non_ascii(text).into_iter().chain(TAG_GLYPHS.chars()) {
            for (what, id) in [("proportional", &body), ("medium", &medium)] {
                if !f.has_glyph(id, c) {
                    missing.push(format!("{c:?} ({what})"));
                }
            }
        }
        for c in icons::ALL {
            if !f.has_glyph(&symbols, *c) {
                missing.push(format!("{c:?} (symbols)"));
            }
        }
    });
    assert!(missing.is_empty(), "no glyph for: {missing:?}");
}

#[test]
fn icons_are_distinct_private_use_chars() {
    let unique: BTreeSet<char> = icons::ALL.iter().copied().collect();
    assert_eq!(unique.len(), icons::ALL.len());
    assert!(
        icons::ALL
            .iter()
            .all(|c| ('\u{e000}'..='\u{f8ff}').contains(c))
    );
}

#[test]
fn icon_uses_the_symbols_family_at_the_given_size() {
    let rich = icons::icon(icons::GEAR, 20.0);
    let ctx = ready_context();
    let job = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let galley = egui::WidgetText::from(rich.clone()).into_galley(
                ui,
                None,
                f32::INFINITY,
                TextStyle::Body,
            );
            let font = &galley.job.sections[0].format.font_id;
            assert_eq!(font.family, FontFamily::Name(SYMBOLS_FAMILY.into()));
            assert_eq!(font.size, 20.0);
        });
    });
    drop(job);
}

#[test]
fn roboto_text_style_uses_roboto() {
    let defs = font_definitions();
    let proportional = &defs.families[&FontFamily::Proportional];
    assert_eq!(proportional[0], "roboto-regular");
    assert!(proportional.len() > 1, "egui's defaults stay as fallback");
    assert_eq!(
        defs.families[&FontFamily::Name(MEDIUM_FAMILY.into())][0],
        "roboto-medium"
    );
    assert_eq!(
        defs.families[&FontFamily::Name(SYMBOLS_FAMILY.into())],
        vec!["symbols".to_owned()]
    );
    for font in typography::text_styles().values() {
        if font.family != FontFamily::Monospace {
            assert_eq!(font.family, FontFamily::Proportional);
        }
    }
}

#[test]
fn roboto_is_wider_than_the_egui_default_for_the_same_text() {
    // Roboto's "Connections" at 14pt must come from Roboto, not Ubuntu-Light: the widths differ.
    let ctx = ready_context();
    let id = FontId::new(14.0, FontFamily::Proportional);
    let roboto = ctx.fonts(|f| f.glyph_width(&id, 'm'));
    let default_ctx = Context::default();
    let _ = default_ctx.run(egui::RawInput::default(), |_| {});
    let default = default_ctx.fonts(|f| f.glyph_width(&id, 'm'));
    assert_ne!(roboto, default);
}

#[test]
fn the_glyph_check_can_fail() {
    // Negative control: a codepoint no font has, and an icon asked of the text family.
    let ctx = ready_context();
    ctx.fonts(|f| {
        let body = FontId::new(14.0, FontFamily::Proportional);
        let symbols = FontId::new(24.0, FontFamily::Name(SYMBOLS_FAMILY.into()));
        assert!(!f.has_glyph(&body, '\u{10fffd}'));
        assert!(!f.has_glyph(&symbols, 'A'));
    });
}
