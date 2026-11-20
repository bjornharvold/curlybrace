//! How two panes share the central area, decided from its size alone (#159). Pure.
//!
//! The panes sit side by side when each gets room for the Browse page, else one above the other
//! when each does, else only one pane is shown with a chip that says so. "Room for the page" is
//! the measured staircase [`STAIRS`]: a narrower pane needs a taller one, because its find bar
//! and toolbar wrap into more rows. The person's chosen orientation is used whenever it fits.
use eframe::egui::Vec2;

/// The divider's hit area: the room between the two panes (points).
pub const DIVIDER: f32 = 8.0;
/// The narrowest a pane may be at all (points), whatever its height.
pub const MIN_PANE_WIDTH: f32 = 400.0;
/// The lowest a pane may be at all (points), whatever its width.
pub const MIN_PANE_HEIGHT: f32 = 250.0;
/// The corners `(width, height)` of the smallest panes the Browse page is usable in, measured on
/// the real page (`tests/split_layout.rs::pane_minimums_match_the_measured_page`), with a real
/// connection name: the header (name, badge, Documents | Indexes | Aggregate) and the find bar
/// whole, none squashed, nothing past the edge or overlapping, and three documents in view. A
/// narrower pane wraps its find bar into more rows, so it needs more height. A pane fits when it
/// is at least as wide and as high as one corner. The two corners that decide 1000x700 were
/// measured at 1 pt (608 and 695), the others at 10 pt.
///
/// The decision has no hysteresis on purpose: it reads the central area only, and that area is the
/// same whether one pane or two are drawn (the chip is placed after the decision), so a layout
/// cannot feed back into its own input and flicker; a window dragged across a threshold changes
/// once.
pub const STAIRS: [(f32, f32); 6] = [
    (400.0, 430.0),
    (470.0, 380.0),
    (500.0, 350.0),
    (608.0, 330.0),
    (695.0, 280.0),
    (960.0, 250.0),
];
/// The ratio a divider is reset to.
pub const DEFAULT_RATIO: f32 = 0.5;
/// How far the divider moves for one arrow key press.
pub const RATIO_STEP: f32 = 0.05;
const RATIO_MIN: f32 = 0.25;
const RATIO_MAX: f32 = 0.75;

/// How the two panes are placed when there is room for both.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Orientation {
    /// One left of the other.
    #[default]
    SideBySide,
    /// One above the other.
    Stacked,
}

/// What the central area shows. `ratio` is the first pane's share of the room the panes have
/// (the area less the divider).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SplitLayout {
    Side {
        ratio: f32,
    },
    Stacked {
        ratio: f32,
    },
    /// No room for two panes: one pane and a chip.
    Single,
}

/// A ratio within what the divider may take (a quarter to three quarters).
pub fn clamped(ratio: f32) -> f32 {
    match ratio.is_finite() {
        true => ratio.clamp(RATIO_MIN, RATIO_MAX),
        false => DEFAULT_RATIO,
    }
}

/// The narrowest a pane `height` high may be, when it fits at all.
pub fn min_width_at(height: f32) -> Option<f32> {
    STAIRS
        .iter()
        .filter(|(_, h)| height >= *h)
        .map(|(w, _)| *w)
        .reduce(f32::min)
}

/// The lowest a pane `width` wide may be, when it fits at all.
pub fn min_height_at(width: f32) -> Option<f32> {
    STAIRS
        .iter()
        .filter(|(w, _)| width >= *w)
        .map(|(_, h)| *h)
        .reduce(f32::min)
}

/// The ratio kept inside what leaves both panes `min` of the `room` they share.
fn within_room(ratio: f32, room: f32, min: f32) -> f32 {
    let floor = (min / room).clamp(0.0, 0.5);
    clamped(ratio).clamp(floor, 1.0 - floor)
}

/// The ratio for two panes side by side in `central`, when both fit.
fn side(central: Vec2, ratio: f32) -> Option<f32> {
    let room = central.x - DIVIDER;
    // What the page gets of a pane is less its ring on every side.
    let min = min_width_at(central.y)?;
    (room / 2.0 >= min).then(|| within_room(ratio, room, min))
}

/// The ratio for two panes one above the other in `central`, when both fit.
fn stacked(central: Vec2, ratio: f32) -> Option<f32> {
    let room = central.y - DIVIDER;
    let min = min_height_at(central.x)?;
    (room / 2.0 >= min).then(|| within_room(ratio, room, min))
}

/// The least and the most the divider can take in `central` as it is laid out: a quarter to three
/// quarters, less what the other pane's minimum leaves. (One pane: the plain limits.)
pub fn ratio_limits(central: Vec2, preferred: Orientation) -> (f32, f32) {
    let at = |ratio: f32| match layout(central, preferred, ratio) {
        SplitLayout::Side { ratio } | SplitLayout::Stacked { ratio } => ratio,
        SplitLayout::Single => clamped(ratio),
    };
    (at(0.0), at(1.0))
}

/// The layout of `central` (the area under the tab strip) for the chosen orientation.
pub fn layout(central: Vec2, preferred: Orientation, ratio: f32) -> SplitLayout {
    let (side, stacked) = (side(central, ratio), stacked(central, ratio));
    let (first, second) = match preferred {
        Orientation::SideBySide => (
            side.map(|ratio| SplitLayout::Side { ratio }),
            stacked.map(|ratio| SplitLayout::Stacked { ratio }),
        ),
        Orientation::Stacked => (
            stacked.map(|ratio| SplitLayout::Stacked { ratio }),
            side.map(|ratio| SplitLayout::Side { ratio }),
        ),
    };
    first.or(second).unwrap_or(SplitLayout::Single)
}

#[cfg(test)]
#[path = "split_layout_tests.rs"]
mod tests;
