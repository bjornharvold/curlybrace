//! The theme gallery: every custom widget in every state (enabled, hover, pressed, focused,
//! disabled, and the error tone) in both modes, rendered with wgpu and compared with
//! `tests/snapshots/theme_gallery_{light,dark}.png`. A token change is reviewed here in one place.
//!
//! Hover, pressed and focused are forced per column with `theme::widgets::with_forced`, so the
//! picture does not depend on where a simulated pointer happens to be.
//!
//! Run:    `CURLYBRACE_SNAPSHOTS=1 cargo test -p curlybrace-ui --test theme_gallery -- --include-ignored`
//! Update: add `UPDATE_SNAPSHOTS=1` (deliberately, then review the PNG diff).
#[path = "support/snapshot_options.rs"]
mod snapshot_options;
#[path = "support/widget_gallery.rs"]
mod widget_gallery;

use curlybrace_testkit::snapshots_enabled;
use curlybrace_ui::theme::Mode;
use eframe::egui;
use egui_kittest::Harness;
use snapshot_options::snapshot_options;
use widget_gallery::{GalleryState, gallery, themed};

const SIZE: (f32, f32) = (1160.0, 780.0);

fn render(mode: Mode, name: &str) {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(SIZE.0, SIZE.1))
        .with_pixels_per_point(1.0)
        .wgpu()
        .build_state(themed_panel(mode), GalleryState::default());
    harness.run();
    harness.snapshot_options(name, &snapshot_options());
}

/// The gallery inside a central panel, so the background is the app's panel fill.
fn themed_panel(mode: Mode) -> impl FnMut(&egui::Context, &mut GalleryState) {
    let mut body = themed(mode, |ui, state: &mut GalleryState| gallery(ui, state));
    move |ctx, state| {
        egui::CentralPanel::default().show(ctx, |ui| body(ui, state));
    }
}

#[test]
#[ignore = "needs a GPU: set CURLYBRACE_SNAPSHOTS=1"]
fn theme_gallery_light() {
    if !snapshots_enabled("theme_gallery_light") {
        return;
    }
    render(Mode::Light, "theme_gallery_light");
}

#[test]
#[ignore = "needs a GPU: set CURLYBRACE_SNAPSHOTS=1"]
fn theme_gallery_dark() {
    if !snapshots_enabled("theme_gallery_dark") {
        return;
    }
    render(Mode::Dark, "theme_gallery_dark");
}
