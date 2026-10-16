//! How the screenshot suite compares a render with its committed PNG. Kept apart from the
//! fixtures so a test of the comparison itself does not compile the whole app harness.
use egui_kittest::SnapshotOptions;

/// Differing pixels tolerated per snapshot: none. Even a short label (six characters changed
/// fewer than a hundred pixels) must fail, and a forced `UPDATE_SNAPSHOTS=1` only rewrites a
/// snapshot whose difference is above this threshold. The CI job renders on the same kind of
/// machine as the committed PNGs (macOS on Apple silicon; `ui-snapshots` passed at 0 on
/// `macos-14`); if another GPU or driver ever adds anti-aliasing noise, make this per OS
/// (`OsThreshold`) instead of loosening it everywhere. kittest's comparison still skips pixels
/// it classes as anti-aliasing, so a change that only moves glyph edges can go unseen.
pub const PIXEL_THRESHOLD: usize = 0;

/// The options every snapshot of the suite is compared with.
pub fn snapshot_options() -> SnapshotOptions {
    SnapshotOptions::new().failed_pixel_count_threshold(PIXEL_THRESHOLD)
}
