//! Material Symbols (Outlined, weight 400, fill 0, 24 opsz) codepoints in the bundled subset.
//! Keep in sync with the glyph list in `scripts/subset-icons.sh`.
use super::fonts::SYMBOLS_FAMILY;
use eframe::egui::{FontFamily, FontId, RichText};

pub const GEAR: char = '\u{e8b8}';
pub const CHECK: char = '\u{e5ca}';
pub const CHEVRON_RIGHT: char = '\u{e5cc}';
pub const CHEVRON_DOWN: char = '\u{e5cf}';
pub const CLOSE: char = '\u{e5cd}';
pub const WARNING: char = '\u{e002}';
pub const ERROR: char = '\u{e000}';
pub const INFO: char = '\u{e88e}';
pub const ARROW_UP: char = '\u{e5d8}';
pub const ARROW_DOWN: char = '\u{e5db}';

/// Every icon in the subset.
pub const ALL: &[char] = &[
    GEAR,
    CHECK,
    CHEVRON_RIGHT,
    CHEVRON_DOWN,
    CLOSE,
    WARNING,
    ERROR,
    INFO,
    ARROW_UP,
    ARROW_DOWN,
];

/// `c` drawn from the icon font at `size` points.
pub fn icon(c: char, size: f32) -> RichText {
    RichText::new(c.to_string()).font(FontId::new(size, FontFamily::Name(SYMBOLS_FAMILY.into())))
}
