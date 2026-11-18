//! How tall the scrolling part of a dialog may be. A dialog is centred and does not scroll the
//! window, so its body gives way on a short window: the window's height less the `chrome` (what
//! the dialog draws around the body: title, footer, frame margins), kept between `min` and
//! `max`. The chrome is a measured constant per dialog; `tests/layout_audit_800.rs` proves each
//! dialog still fits an 800x500 window with it.
use eframe::egui;

/// The body's height for a window `window` points tall.
pub(super) fn body_height_in(window: f32, chrome: f32, min: f32, max: f32) -> f32 {
    (window - chrome).max(min).min(max)
}

/// [`body_height_in`] for the window of `ctx`.
pub(super) fn body_max_height(ctx: &egui::Context, chrome: f32, min: f32, max: f32) -> f32 {
    body_height_in(ctx.screen_rect().height(), chrome, min, max)
}

#[cfg(test)]
mod tests {
    use super::body_height_in;

    #[test]
    fn a_tall_window_gives_the_maximum() {
        assert_eq!(body_height_in(1400.0, 250.0, 120.0, 460.0), 460.0);
    }

    #[test]
    fn a_short_window_gives_what_is_left() {
        assert_eq!(body_height_in(500.0, 250.0, 120.0, 460.0), 250.0);
    }

    #[test]
    fn a_window_too_short_for_the_chrome_keeps_the_minimum() {
        assert_eq!(body_height_in(300.0, 250.0, 120.0, 460.0), 120.0);
    }

    #[test]
    fn an_unbounded_maximum_is_allowed() {
        assert_eq!(body_height_in(900.0, 250.0, 120.0, f32::INFINITY), 650.0);
    }
}
