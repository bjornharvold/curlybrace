//! The bundled font stack: Roboto (Regular and Medium) and a Material Symbols subset.
use eframe::egui::{FontData, FontDefinitions, FontFamily};
use std::sync::Arc;

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
