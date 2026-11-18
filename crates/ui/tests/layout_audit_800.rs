//! An audit of every page at the smallest supported window, 800x500, with 28 pt controls: on each
//! page no two controls overlap, none runs past the window's right edge, none is squashed below
//! a usable size, and the page's first control row is on screen. Measured from accesskit bounds
//! of the REAL app on an in-memory store. Fast, offline, no GPU. Run with `--nocapture` for the
//! measurements (the tallest and the smallest control of each page).
mod support;

use curlybrace_core::connection::ConnectionConfig;
use curlybrace_ui::I18n;
use curlybrace_ui::app::CurlybraceApp;
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::theme;
use eframe::egui::{self, Rect, accesskit::Role};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use support::aggregate_shots::aggregate_tab;
use support::compare_states::{Through, compare_state};
use support::indexes_shots::{indexes_tab, queries_view};
use support::shell_shots::empty as shell_empty;
use support::shots::{browsing, build_app};

const SMALL: egui::Vec2 = egui::vec2(800.0, 500.0);
/// The smallest a button, field or tab may be, tall (points): the 28 pt controls less a few
/// points for the compact 24 pt segmented control and the check boxes.
const MIN_CONTROL_HEIGHT: f32 = 14.0;
/// Overlaps smaller than this are rounding, not layout (square points).
const OVERLAP_TOLERANCE: f32 = 1.0;

struct Page {
    harness: Harness<'static, CurlybraceApp>,
    _rt: tokio::runtime::Runtime,
    _dir: tempfile::TempDir,
}

fn page(connections: Vec<ConnectionConfig>, setup: impl FnOnce(&mut AppState)) -> Page {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .build()
        .expect("tokio runtime");
    let enter = rt.enter();
    let (mut app, dir) = build_app(connections, None);
    setup(app.state_mut());
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(SMALL)
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
    harness.run_steps(8);
    drop(enter);
    Page {
        harness,
        _rt: rt,
        _dir: dir,
    }
}

fn is_control(role: Role) -> bool {
    matches!(
        role,
        Role::Button
            | Role::CheckBox
            | Role::RadioButton
            | Role::ComboBox
            | Role::TextInput
            | Role::MultilineTextInput
    )
}

/// The rect of every control on the page (name, rect) that is drawn at all.
fn controls(page: &Page) -> Vec<(String, Rect)> {
    page.harness
        .root()
        .children_recursive()
        .filter(|n| is_control(n.accesskit_node().role()))
        .map(|n| {
            let name = n.accesskit_node().label().unwrap_or_default();
            (format!("{:?} {name}", n.accesskit_node().role()), n.rect())
        })
        .filter(|(_, rect)| rect.width() > 0.0 && rect.height() > 0.0)
        .collect()
}

fn audit(name: &str, page: &Page) {
    audit_without(name, page, |_| false);
}

/// The audit of a dialog: its buttons (the footer, which never scrolls) are all on screen, none
/// overlaps another or runs past the right edge. A dialog's body may scroll, so its fields and
/// check boxes can lie outside the window and their unclipped rects overlap the footer's.
fn audit_dialog(name: &str, base: Setup, page: &Page) {
    // The controls of the page behind the dialog (the sidebar, the page) are not the dialog's;
    // they are told apart by name.
    let behind = controls(&self::page(vec![], base));
    let buttons: Vec<_> = controls(page)
        .into_iter()
        .filter(|(label, _)| !behind.iter().any(|(other, _)| other == label))
        .filter(|(label, _)| label.starts_with("Button "))
        .collect();
    eprintln!(
        "{name}: {} buttons, lowest bottom {}",
        buttons.len(),
        buttons.iter().map(|(_, r)| r.bottom()).fold(0.0, f32::max)
    );
    for (label, rect) in &buttons {
        assert!(
            rect.bottom() <= SMALL.y + 0.5 && rect.top() >= 0.0,
            "{name}: {label} leaves the window: {rect:?}"
        );
    }
    audit_controls(name, buttons);
}

/// [`audit`] without the controls `scrolled` names: content of a scroll area, whose rect lies
/// outside the part of the area that is visible (an accessibility rect is not clipped).
fn audit_without(name: &str, page: &Page, scrolled: impl Fn(&str) -> bool) {
    let shown: Vec<_> = controls(page)
        .into_iter()
        .filter(|(label, _)| !scrolled(label))
        .collect();
    audit_controls(name, shown);
}

fn audit_controls(name: &str, shown: Vec<(String, Rect)>) {
    assert!(!shown.is_empty(), "{name}: no controls found");
    let on_screen: Vec<_> = shown
        .iter()
        .filter(|(_, rect)| rect.top() < SMALL.y)
        .collect();
    let smallest = on_screen
        .iter()
        .map(|(_, r)| r.height())
        .fold(f32::MAX, f32::min);
    let tallest = on_screen
        .iter()
        .map(|(_, r)| r.height())
        .fold(0.0, f32::max);
    let lowest = on_screen
        .iter()
        .map(|(_, r)| r.bottom())
        .fold(0.0, f32::max);
    eprintln!(
        "{name}: {} controls, {} on screen, heights {smallest}..{tallest}, lowest bottom {lowest}",
        shown.len(),
        on_screen.len()
    );
    for (label, rect) in &on_screen {
        assert!(
            rect.right() <= SMALL.x + 0.5,
            "{name}: {label} runs past the right edge: {rect:?}"
        );
        assert!(
            rect.height() >= MIN_CONTROL_HEIGHT,
            "{name}: {label} is squashed: {rect:?}"
        );
    }
    for (i, (a, ra)) in on_screen.iter().enumerate() {
        for (b, rb) in &on_screen[i + 1..] {
            let overlap = ra.intersect(*rb);
            assert!(
                !overlap.is_positive() || overlap.area() < OVERLAP_TOLERANCE,
                "{name}: {a} {ra:?} overlaps {b} {rb:?}"
            );
        }
    }
}

fn two_connections() -> Vec<ConnectionConfig> {
    vec![
        ConnectionConfig::new("Production replica", "mongodb://db.example.com:27017"),
        ConnectionConfig::new("Staging", "mongodb://staging.example.com:27017"),
    ]
}

#[test]
fn browse_documents() {
    audit(
        "browse",
        &page(vec![], browsing(curlybrace_ui::viewmodel::ViewMode::Table)),
    );
}

#[test]
fn indexes() {
    audit("indexes", &page(vec![], indexes_tab));
}

#[test]
fn queries() {
    audit("queries", &page(vec![], queries_view));
}

#[test]
fn aggregate() {
    // The stage list scrolls inside half of the room: only the first card is fully shown.
    let later_stages = |label: &str| {
        let label = label.to_lowercase();
        label.contains("stage 2") || label.contains("stage 3")
    };
    audit_without("aggregate", &page(vec![], aggregate_tab), later_stages);
}

#[test]
fn shell() {
    audit("shell", &page(vec![], shell_empty));
}

#[test]
fn compare_setup() {
    audit(
        "compare empty",
        &page(two_connections(), compare_state(Through::Empty)),
    );
    audit(
        "compare pairs",
        &page(two_connections(), compare_state(Through::Blocked)),
    );
}

#[test]
fn compare_results() {
    audit(
        "compare results",
        &page(two_connections(), compare_state(Through::Results)),
    );
}

#[test]
fn connections_manager() {
    let i18n = I18n::english();
    // The manager is the start screen while nothing is connected.
    let page = page(two_connections(), |_| {});
    page.harness
        .get_by_role_and_label(Role::TextInput, &i18n.t("connections.filter"));
    audit("connections", &page);
}

type Setup = fn(&mut AppState);

fn browse_table(state: &mut AppState) {
    browsing(curlybrace_ui::viewmodel::ViewMode::Table)(state);
}

/// One test per dialog or notice over the page that a window of 800x500 has to hold: `$base`
/// is the page behind it.
macro_rules! dialog_fits {
    ($name:ident, $base:expr, $setup:expr) => {
        #[test]
        fn $name() {
            audit_dialog(stringify!($name), $base, &page(vec![], $setup));
        }
    };
}

mod dialogs {
    use super::*;
    use support::{bulk_shots, code_export_shots, copy_shots, production_shots};

    dialog_fits!(bulk_define, browse_table, bulk_shots::define_update);
    dialog_fits!(
        bulk_preview_update,
        browse_table,
        bulk_shots::preview_update
    );
    dialog_fits!(
        bulk_preview_delete,
        browse_table,
        bulk_shots::preview_delete
    );
    dialog_fits!(
        bulk_confirm_all,
        browse_table,
        bulk_shots::confirm_all_documents
    );
    dialog_fits!(bulk_running, browse_table, bulk_shots::running);
    dialog_fits!(bulk_report, browse_table, bulk_shots::report_applied);
    dialog_fits!(
        bulk_report_undo,
        browse_table,
        bulk_shots::report_undo_offered
    );
    dialog_fits!(bulk_undo_confirm, browse_table, bulk_shots::undo_confirm);
    dialog_fits!(copy_counting, browse_table, copy_shots::dialog_counting);
    dialog_fits!(
        copy_overwrite_production,
        browse_table,
        copy_shots::dialog_overwrite_production
    );
    dialog_fits!(copy_progress, browse_table, copy_shots::progress);
    dialog_fits!(copy_report, browse_table, copy_shots::report_indexes);
    dialog_fits!(
        code_export,
        aggregate_tab,
        code_export_shots::code_export_dialog
    );
    dialog_fits!(
        code_export_options,
        aggregate_tab,
        code_export_shots::code_export_dialog_options
    );
    dialog_fits!(
        index_builder,
        indexes_tab,
        support::indexes_shots::index_builder
    );
    dialog_fits!(
        index_drop,
        indexes_tab,
        support::indexes_shots::index_drop_confirm
    );
    dialog_fits!(
        production_warning,
        browse_table,
        production_shots::write_warning
    );
}
