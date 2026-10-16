//! The snapshot suite's comparison is strict enough to catch a changed short label. Runs without
//! a GPU: synthetic images stand in for a rendered screen, written to a temp dir.
mod support;

use egui_kittest::{SnapshotError, try_image_snapshot_options};
use image::{Rgba, RgbaImage};
use support::shots::snapshot_options;

const NAME: &str = "screen";

fn screen() -> RgbaImage {
    RgbaImage::from_pixel(200, 100, Rgba([30, 30, 30, 255]))
}

/// A label's worth of glyph pixels (a six character word covers roughly this many).
fn with_label_changed(mut image: RgbaImage, pixels: u32) -> RgbaImage {
    for i in 0..pixels {
        image.put_pixel(10 + i % 40, 20 + i / 40, Rgba([240, 240, 240, 255]));
    }
    image
}

fn compare(committed: &RgbaImage, rendered: &RgbaImage) -> Result<(), SnapshotError> {
    let dir = tempfile::tempdir().expect("temp dir");
    committed
        .save(dir.path().join(format!("{NAME}.png")))
        .expect("write the committed snapshot");
    let options = snapshot_options().output_path(dir.path());
    try_image_snapshot_options(rendered, NAME, &options)
}

#[test]
fn an_identical_render_passes() {
    assert!(compare(&screen(), &screen()).is_ok());
}

#[test]
fn a_short_label_change_fails() {
    for pixels in [1, 6, 60, 99] {
        let result = compare(&screen(), &with_label_changed(screen(), pixels));
        assert!(
            matches!(result, Err(SnapshotError::Diff { .. })),
            "{pixels} differing pixels passed: {result:?}"
        );
    }
}
