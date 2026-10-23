//! How a connection's colour tag and production flag are shown. Colour is never the only
//! signal: every colour has its own glyph and a translated name, and production is a text badge.
use curlybrace_core::connection::{ConnectionColor, ConnectionConfig};

/// i18n key of the production badge text.
pub const PRODUCTION_BADGE_KEY: &str = "connection.production_badge";

/// What draws one colour: its rgb (readable on egui's light and dark themes), a glyph that
/// differs per colour (and exists in egui's default fonts, which a test checks) and the i18n key of its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorTag {
    pub rgb: [u8; 3],
    pub glyph: &'static str,
    pub name_key: &'static str,
}

/// What to show next to a connection's name.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConnectionBadge {
    pub color: Option<ColorTag>,
    pub production: bool,
}

pub fn color_tag(c: ConnectionColor) -> ColorTag {
    let (rgb, glyph, name_key) = match c {
        ConnectionColor::Red => ([214, 48, 49], "♥", "connection.color.red"),
        ConnectionColor::Orange => ([208, 112, 4], "▶", "connection.color.orange"),
        ConnectionColor::Yellow => ([170, 124, 0], "■", "connection.color.yellow"),
        ConnectionColor::Green => ([46, 160, 67], "♣", "connection.color.green"),
        ConnectionColor::Blue => ([49, 120, 220], "★", "connection.color.blue"),
        ConnectionColor::Purple => ([150, 85, 200], "♦", "connection.color.purple"),
    };
    ColorTag {
        rgb,
        glyph,
        name_key,
    }
}

impl ConnectionBadge {
    pub fn new(color: Option<ConnectionColor>, production: bool) -> Self {
        Self {
            color: color.map(color_tag),
            production,
        }
    }
}

pub fn badge_of(cfg: &ConnectionConfig) -> ConnectionBadge {
    ConnectionBadge::new(cfg.color, cfg.production)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui;
    use std::collections::BTreeSet;

    fn all_tags() -> Vec<ColorTag> {
        ConnectionColor::ALL.into_iter().map(color_tag).collect()
    }

    #[test]
    fn every_color_has_a_distinct_glyph_and_name_key() {
        let tags = all_tags();
        let glyphs: BTreeSet<_> = tags.iter().map(|t| t.glyph).collect();
        let keys: BTreeSet<_> = tags.iter().map(|t| t.name_key).collect();
        assert_eq!(glyphs.len(), 6);
        assert_eq!(keys.len(), 6);
    }

    #[test]
    fn every_color_name_key_exists_in_en_json() {
        let i18n = crate::I18n::english();
        for tag in all_tags() {
            assert!(!i18n.t(tag.name_key).starts_with('⟦'), "{}", tag.name_key);
        }
        assert!(!i18n.t(PRODUCTION_BADGE_KEY).starts_with('⟦'));
    }

    fn has_glyph_in_default_fonts(glyphs: &[&str]) -> Vec<(String, bool)> {
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |_| {});
        let font = egui::FontId::proportional(14.0);
        glyphs
            .iter()
            .map(|g| {
                let ok = g.chars().all(|c| ctx.fonts(|f| f.has_glyph(&font, c)));
                (g.to_string(), ok)
            })
            .collect()
    }

    #[test]
    fn every_glyph_exists_in_the_default_fonts() {
        let glyphs: Vec<&str> = all_tags().iter().map(|t| t.glyph).collect();
        for (glyph, ok) in has_glyph_in_default_fonts(&glyphs) {
            assert!(ok, "{glyph} is missing from egui's default fonts");
        }
    }

    fn luminance(rgb: [u8; 3]) -> f64 {
        let lin = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(rgb[0]) + 0.7152 * lin(rgb[1]) + 0.0722 * lin(rgb[2])
    }

    fn contrast(a: [u8; 3], b: [u8; 3]) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn every_color_has_graphic_contrast_on_light_and_dark_themes() {
        for tag in all_tags() {
            for background in [[248, 248, 248], [27, 27, 27]] {
                let ratio = contrast(tag.rgb, background);
                assert!(ratio >= 3.0, "{} on {background:?}: {ratio}", tag.name_key);
            }
        }
    }

    #[test]
    fn badge_of_reflects_config() {
        let plain = ConnectionConfig::new("a", "mongodb://h:1");
        assert_eq!(badge_of(&plain), ConnectionBadge::default());
        let mut tagged = plain;
        tagged.color = Some(ConnectionColor::Green);
        tagged.production = true;
        let badge = badge_of(&tagged);
        assert_eq!(badge.color, Some(color_tag(ConnectionColor::Green)));
        assert!(badge.production);
    }
}
