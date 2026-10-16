//! The snapshot suite's comparison is strict enough to catch a changed short label, and a forced
//! update rewrites such a snapshot. Runs without a GPU: synthetic images stand in for a rendered
//! screen, written to a temp dir. Only the comparison options are compiled in, not the app harness.
#[path = "support/snapshot_options.rs"]
mod snapshot_options;

use egui_kittest::{SnapshotError, try_image_snapshot_options};
use image::{Rgba, RgbaImage};
use snapshot_options::snapshot_options;
use std::path::Path;
use std::sync::Mutex;

const NAME: &str = "screen";

/// `UPDATE_SNAPSHOTS` is process-wide and read on every comparison, so tests take turns.
static ENV: Mutex<()> = Mutex::new(());

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

/// Compares `rendered` with a committed `committed` in `dir`, with `UPDATE_SNAPSHOTS` set to
/// `update` (or removed) for the duration.
fn compare(
    dir: &Path,
    committed: &RgbaImage,
    rendered: &RgbaImage,
    update: bool,
) -> Result<(), SnapshotError> {
    let _turn = ENV.lock().unwrap_or_else(|e| e.into_inner());
    committed
        .save(dir.join(format!("{NAME}.png")))
        .expect("write the committed snapshot");
    // SAFETY: every test that reads the variable holds `ENV`.
    unsafe {
        if update {
            std::env::set_var("UPDATE_SNAPSHOTS", "1");
        } else {
            std::env::remove_var("UPDATE_SNAPSHOTS");
        }
    }
    let options = snapshot_options().output_path(dir);
    let result = try_image_snapshot_options(rendered, NAME, &options);
    unsafe { std::env::remove_var("UPDATE_SNAPSHOTS") };
    result
}

#[test]
fn an_identical_render_passes() {
    let dir = tempfile::tempdir().expect("temp dir");
    assert!(compare(dir.path(), &screen(), &screen(), false).is_ok());
}

#[test]
fn a_short_label_change_fails() {
    for pixels in [1, 6, 60, 99] {
        let dir = tempfile::tempdir().expect("temp dir");
        let result = compare(
            dir.path(),
            &screen(),
            &with_label_changed(screen(), pixels),
            false,
        );
        assert!(
            matches!(result, Err(SnapshotError::Diff { .. })),
            "{pixels} differing pixels passed: {result:?}"
        );
    }
}

#[test]
fn a_forced_update_rewrites_a_short_label_change() {
    for pixels in [1, 6, 99] {
        let dir = tempfile::tempdir().expect("temp dir");
        let changed = with_label_changed(screen(), pixels);
        compare(dir.path(), &screen(), &changed, true).expect("forced update succeeds");
        let rewritten = image::open(dir.path().join(format!("{NAME}.png")))
            .expect("snapshot")
            .to_rgba8();
        assert_eq!(
            rewritten, changed,
            "{pixels} differing pixels not rewritten"
        );
    }
}
