//! The Browse page of the REAL app on an in-memory store at a chosen window size, for the layout
//! tests of the header: no GPU, no database. The page shows the `browsing` fixture (a sample
//! page of people), light theme, one point per pixel, six frames so the measured widths settle.
use super::shots::{browsing, build_app};
use curlybrace_ui::I18n;
use curlybrace_ui::app::CurlybraceApp;
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::theme;
use curlybrace_ui::viewmodel::{FindBarState, ViewMode};
use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

pub const SIZE_1000: (f32, f32) = (1000.0, 700.0);
pub const SIZE_800: (f32, f32) = (800.0, 500.0);

// Where the first document row may start at most (points from the window's top); the budgets of
// the spec, shared by every layout test that guards the header.

/// What sits above the first row at 1000x700 with More closed: the connection and tabs row, the
/// Filter row, the switches with the verdict beside them, the documents toolbar (two rows) and
/// the table's column header.
pub const CLOSED_1000: f32 = 228.0;
/// The same with More open: the Sort / Projection / Skip / Limit / Query options row (one row).
pub const MORE_1000: f32 = 275.0;
/// With More and the Query options section open (nothing set): the section's share of the window
/// is on top of [`MORE_1000`]; the first row stays on screen (700 high).
pub const MORE_AND_OPTIONS_1000: f32 = 545.0;
/// At 800x500 with More closed: the Filter row and the switches wrap to two rows each.
pub const CLOSED_800: f32 = 296.0;
/// At 800x500 with More open: the More row wraps to two rows; the first row is on screen
/// (500 high) with room for at least four rows under it (a row is about 18 pt plus the 4 pt gap).
pub const MORE_800: f32 = 390.0;

/// The page with the runtime and the temp dir it needs alive.
pub struct BrowsePage {
    pub harness: Harness<'static, CurlybraceApp>,
    _rt: tokio::runtime::Runtime,
    _dir: tempfile::TempDir,
}

/// The Browse page at `size` with `adjust` applied to the state after the page is installed.
pub fn browse_page(size: (f32, f32), adjust: impl FnOnce(&mut AppState)) -> BrowsePage {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .build()
        .expect("tokio runtime");
    let enter = rt.enter();
    let (mut app, dir) = build_app(vec![], None);
    browsing(ViewMode::Table)(app.state_mut());
    adjust(app.state_mut());
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(size.0, size.1))
        .with_pixels_per_point(1.0)
        .build_state(
            move |ctx, app: &mut CurlybraceApp| {
                if !installed {
                    theme::install(ctx);
                    ctx.set_theme(egui::Theme::Light);
                    installed = true;
                }
                app.run_frame(ctx)
            },
            app,
        );
    harness.run_steps(6);
    drop(enter);
    BrowsePage {
        harness,
        _rt: rt,
        _dir: dir,
    }
}

/// The find bar of a page where nothing is typed: More open or closed, the Query options section
/// open when asked (`options`), nothing set in either.
pub fn bar(more: bool, options: Option<bool>) -> impl FnOnce(&mut AppState) {
    move |state| {
        state.find = FindBarState {
            more: Some(more),
            options_open: options,
            ..FindBarState::default()
        };
    }
}

pub fn t(key: &str) -> String {
    I18n::english().t(key)
}

impl BrowsePage {
    /// The top of the first document row's checkbox ("Select document 1").
    pub fn first_row_top(&self) -> f32 {
        let label = I18n::english().t_with("doc.select_row", &[("n", "1")]);
        self.harness.get_by_label(&label).rect().top()
    }
}
