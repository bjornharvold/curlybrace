//! The bundled font stack: Roboto (Regular and Medium) and a Material Symbols subset.
use eframe::egui::{Context, FontData, FontDefinitions, FontFamily};
use std::sync::{Arc, OnceLock};

/// Family for Roboto Medium (titles, `RichText::strong`).
pub const MEDIUM_FAMILY: &str = "roboto-medium";
/// Family for the Material Symbols icon subset (private-use codepoints, see `icons`).
pub const SYMBOLS_FAMILY: &str = "symbols";

const REGULAR_NAME: &str = "roboto-regular";
const ROBOTO_REGULAR: &[u8] = include_bytes!("../../assets/fonts/Roboto-Regular.ttf");
const ROBOTO_MEDIUM: &[u8] = include_bytes!("../../assets/fonts/Roboto-Medium.ttf");
const SYMBOLS: &[u8] = include_bytes!("../../assets/fonts/MaterialSymbolsSubset.ttf");

/// Roboto first in `Proportional` with egui's defaults behind it as fallback (emoji and
/// symbols that appear in data still render); Roboto Medium and the icon font as named families.
pub fn font_definitions() -> FontDefinitions {
    let mut defs = FontDefinitions::default();
    for (name, bytes) in [
        (REGULAR_NAME, ROBOTO_REGULAR),
        (MEDIUM_FAMILY, ROBOTO_MEDIUM),
        (SYMBOLS_FAMILY, SYMBOLS),
    ] {
        defs.font_data
            .insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    }
    let fallback = defs.families[&FontFamily::Proportional].clone();
    defs.families.insert(
        FontFamily::Proportional,
        with_first(REGULAR_NAME, &fallback),
    );
    defs.families.insert(
        FontFamily::Name(MEDIUM_FAMILY.into()),
        with_first(MEDIUM_FAMILY, &fallback),
    );
    defs.families.insert(
        FontFamily::Name(SYMBOLS_FAMILY.into()),
        vec![SYMBOLS_FAMILY.to_owned()],
    );
    defs
}

fn with_first(first: &str, rest: &[String]) -> Vec<String> {
    std::iter::once(first.to_owned())
        .chain(rest.iter().cloned())
        .collect()
}

/// The `Name(MEDIUM_FAMILY)` family, built once: `FontFamily::Name` holds an `Arc<str>`, so
/// building it per call would allocate on every button and pill drawn.
pub(crate) fn medium_family() -> &'static FontFamily {
    static FAMILY: OnceLock<FontFamily> = OnceLock::new();
    FAMILY.get_or_init(|| FontFamily::Name(MEDIUM_FAMILY.into()))
}

/// The `Name(SYMBOLS_FAMILY)` family, built once (see [`medium_family`]).
pub(crate) fn symbols_family() -> &'static FontFamily {
    static FAMILY: OnceLock<FontFamily> = OnceLock::new();
    FAMILY.get_or_init(|| FontFamily::Name(SYMBOLS_FAMILY.into()))
}

/// Whether `family` is among the installed fonts. A bare test context without
/// [`super::install`] lacks the theme's families, and egui panics on an unknown one. This is a
/// map lookup under the font lock: it neither allocates nor builds the family list.
pub(crate) fn has_family(ctx: &Context, family: &FontFamily) -> bool {
    ctx.fonts(|f| f.lock().fonts.definitions().families.contains_key(family))
}
