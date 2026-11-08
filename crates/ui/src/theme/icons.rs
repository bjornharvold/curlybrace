//! Material Symbols (Outlined, weight 400, fill 0, 24 opsz) codepoints in the bundled subset.
//! Keep in sync with the glyph list in `scripts/subset-icons.sh`.
use super::fonts::{has_family, symbols_family};
use super::widgets::medium_font;
use eframe::egui::{
    Align, FontId, FontSelection, RichText, TextStyle, Ui, WidgetText, text::LayoutJob,
};

pub const GEAR: char = '\u{e8b8}';
pub const CHEVRON_RIGHT: char = '\u{e5cc}';
pub const CHEVRON_DOWN: char = '\u{e5cf}';
pub const WARNING: char = '\u{e002}';
pub const ERROR: char = '\u{e000}';
pub const INFO: char = '\u{e88e}';
pub const ARROW_UP: char = '\u{e5d8}';
pub const ARROW_DOWN: char = '\u{e5db}';

/// Every icon in the subset.
pub const ALL: &[char] = &[
    GEAR,
    CHEVRON_RIGHT,
    CHEVRON_DOWN,
    WARNING,
    ERROR,
    INFO,
    ARROW_UP,
    ARROW_DOWN,
];

/// `c` drawn from the icon font at `size` points.
pub fn icon(c: char, size: f32) -> RichText {
    RichText::new(c.to_string()).font(FontId::new(size, symbols_family().clone()))
}

/// The disclosure chevron: down when `open`, right when closed.
pub fn chevron(open: bool) -> char {
    if open { CHEVRON_DOWN } else { CHEVRON_RIGHT }
}

/// [`icon`] at the body text size, for icons that sit inline with text. A context that never
/// ran [`super::install`] (a bare test harness) has no icon family; there the text font is used
/// instead of panicking in egui, so the layout is still exercised.
pub fn inline_icon(ui: &Ui, c: char) -> RichText {
    let size = TextStyle::Body.resolve(ui.style()).size;
    if has_family(ui.ctx(), symbols_family()) {
        icon(c, size)
    } else {
        RichText::new(c.to_string()).size(size)
    }
}

/// `text` in the button font (Roboto Medium), a space and `icon`, as one text for a theme button:
/// a trailing marker such as a sort arrow. The button's accessible name should be set to the
/// plain `text`, since the icon is a private-use codepoint.
pub fn text_then_icon(ui: &Ui, text: &str, icon: char) -> WidgetText {
    let size = TextStyle::Button.resolve(ui.style()).size;
    let style = ui.style();
    let mut job = LayoutJob::default();
    let parts = [
        RichText::new(text).font(medium_font(ui, size)),
        RichText::new(" "),
        inline_icon(ui, icon),
    ];
    for part in parts {
        part.append_to(&mut job, style, FontSelection::Default, Align::Center);
    }
    job.into()
}
