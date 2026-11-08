//! Material 3 type scale mapped onto egui's text styles (sizes in points).
use eframe::egui::{FontFamily, FontId, TextStyle};
use std::collections::BTreeMap;

const LABEL: f32 = 12.0;
const BODY: f32 = 14.0;
const TITLE_M: f32 = 16.0;
const TITLE_L: f32 = 22.0;
const HEADLINE: f32 = 24.0;
const MONO: f32 = 13.0;

/// Material "title medium" (16): section and dialog titles.
pub fn title_medium() -> TextStyle {
    TextStyle::Name("title_medium".into())
}

/// Material "title large" (22): screen titles.
pub fn title_large() -> TextStyle {
    TextStyle::Name("title_large".into())
}

/// egui's styles on the Material scale: Small = label, Body/Button = body, Heading = headline,
/// plus the two title styles above. Monospace stays 13.
pub fn text_styles() -> BTreeMap<TextStyle, FontId> {
    let prop = |size| FontId::new(size, FontFamily::Proportional);
    BTreeMap::from([
        (TextStyle::Small, prop(LABEL)),
        (TextStyle::Body, prop(BODY)),
        (TextStyle::Button, prop(BODY)),
        (title_medium(), prop(TITLE_M)),
        (title_large(), prop(TITLE_L)),
        (TextStyle::Heading, prop(HEADLINE)),
        (
            TextStyle::Monospace,
            FontId::new(MONO, FontFamily::Monospace),
        ),
    ])
}
