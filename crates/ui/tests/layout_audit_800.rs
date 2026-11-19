//! An audit of every page at the smallest supported window, 800x500, with 28 pt controls: on each
//! page no two controls overlap, none runs past the window's right edge, none is squashed below
//! a usable size, and the page's first control row is on screen. Measured from accesskit bounds
//! of the REAL app on an in-memory store. Fast, offline, no GPU. Run with `--nocapture` for the
//! measurements (the tallest and the smallest control of each page).
mod support;

use curlybrace_core::connection::ConnectionConfig;
use curlybrace_ui::I18n;
use curlybrace_ui::app::CurlybraceApp;
use curlybrace_ui::app::host_key::HostKeyRequest;
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::theme;
use curlybrace_ui::viewmodel::split_layout::Orientation;
use eframe::egui::{self, Rect, accesskit::Role};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use support::aggregate_shots::aggregate_tab;
use support::compare_states::{Through, compare_state};
use support::indexes_shots::{indexes_tab, queries_view, queries_view_field_order};
use support::shell_shots::empty as shell_empty;
use support::shots::{browsing, build_app};
use support::tabs_shots::{six_tabs, split_two};

const SMALL: egui::Vec2 = egui::vec2(800.0, 500.0);
/// The smallest a button, field or tab may be, tall (points): the 28 pt controls less a few
/// points for the compact 24 pt segmented control and the check boxes.
const MIN_CONTROL_HEIGHT: f32 = 14.0;
/// A button, a tab, a field is at least this tall (points): WCAG 2.2's 24 pt target.
const MIN_TARGET_HEIGHT: f32 = 24.0;
/// The controls that start within this of the highest one are the page's first row (points).
const FIRST_ROW_DEPTH: f32 = 8.0;
/// Overlaps smaller than this are rounding, not layout (square points).
const OVERLAP_TOLERANCE: f32 = 1.0;

struct Page {
    harness: Harness<'static, CurlybraceApp>,
    _rt: tokio::runtime::Runtime,
    _dir: tempfile::TempDir,
}

fn page(connections: Vec<ConnectionConfig>, setup: impl FnOnce(&mut AppState)) -> Page {
    page_sized(SMALL, connections, setup)
}

fn page_sized(
    size: egui::Vec2,
    connections: Vec<ConnectionConfig>,
    setup: impl FnOnce(&mut AppState),
) -> Page {
    page_full(size, connections, None, setup)
}

fn page_full(
    size: egui::Vec2,
    connections: Vec<ConnectionConfig>,
    requests: Option<std::sync::mpsc::Receiver<HostKeyRequest>>,
    setup: impl FnOnce(&mut AppState),
) -> Page {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .build()
        .expect("tokio runtime");
    let enter = rt.enter();
    let (mut app, dir) = build_app(connections, requests);
    setup(app.state_mut());
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(size)
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
            | Role::Tab
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

/// Buttons (tabs are toggle buttons) and text fields; check boxes, radio buttons and combo boxes
/// keep their own, smaller size.
fn is_target(label: &str) -> bool {
    ["Button ", "Tab ", "TextInput ", "MultilineTextInput "]
        .iter()
        .any(|role| label.starts_with(role))
}

fn audit(name: &str, page: &Page) {
    audit_without(name, page, |_| false);
}

/// The audit of a dialog: its buttons (the footer, which never scrolls) are all on screen, none
/// overlaps another or runs past the right edge. A dialog's body may scroll, so its fields and
/// check boxes can lie outside the window and their unclipped rects overlap the footer's.
fn audit_dialog(name: &str, base: Setup, page: &Page) {
    audit_dialog_against(name, &behind_controls(base), page);
}

/// The controls of the page `base` draws, built once per page for all the dialogs over it.
/// The name and rect of each control of a page.
type Controls = Vec<(String, Rect)>;

fn behind_controls(base: Setup) -> Vec<(String, Rect)> {
    static CACHE: std::sync::Mutex<Vec<(usize, Controls)>> = std::sync::Mutex::new(Vec::new());
    let key = base as usize;
    let mut cache = CACHE.lock().expect("the cache lock");
    if let Some((_, found)) = cache.iter().find(|(k, _)| *k == key) {
        return found.clone();
    }
    let built = controls(&page(two_connections(), base));
    cache.push((key, built.clone()));
    built
}

fn audit_dialog_against(name: &str, behind: &[(String, Rect)], page: &Page) {
    let buttons: Vec<_> = without_page_controls(controls(page), behind)
        .into_iter()
        .filter(|(label, _)| label.starts_with("Button "))
        .collect();
    assert!(!buttons.is_empty(), "{name}: the dialog shows no button");
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

/// `shown` less the controls of the page behind it, one for one: each page control removes the
/// node of the same name that is nearest to it, so a dialog button named like a page button
/// (Copy, Next) stays.
fn without_page_controls(
    mut shown: Vec<(String, Rect)>,
    behind: &[(String, Rect)],
) -> Vec<(String, Rect)> {
    for (label, rect) in behind {
        let nearest = shown
            .iter()
            .enumerate()
            .filter(|(_, (other, _))| other == label)
            .min_by(|(_, (_, a)), (_, (_, b))| {
                let (da, db) = (
                    (a.center() - rect.center()).length(),
                    (b.center() - rect.center()).length(),
                );
                da.total_cmp(&db)
            })
            .map(|(index, _)| index);
        if let Some(index) = nearest {
            shown.remove(index);
        }
    }
    shown
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
    assert!(!on_screen.is_empty(), "{name}: no control is on screen");
    // The first row (the controls that start highest) ends inside the window.
    let first_top = on_screen
        .iter()
        .map(|(_, r)| r.top())
        .fold(f32::MAX, f32::min);
    for (label, rect) in on_screen
        .iter()
        .filter(|(_, r)| r.top() < first_top + FIRST_ROW_DEPTH)
    {
        assert!(
            rect.bottom() <= SMALL.y + 0.5,
            "{name}: {label} of the first row ends below the window: {rect:?}"
        );
    }
    for (label, rect) in &on_screen {
        assert!(
            rect.right() <= SMALL.x + 0.5,
            "{name}: {label} runs past the right edge: {rect:?}"
        );
        let minimum = match is_target(label) {
            true => MIN_TARGET_HEIGHT,
            false => MIN_CONTROL_HEIGHT,
        };
        assert!(
            rect.height() >= minimum,
            "{name}: {label} is squashed below {minimum}: {rect:?}"
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

/// At the standard window the key box and the two filter boxes of the Compare setup share one
/// row (they wrap only when the room is short).
#[test]
fn compare_setup_columns_share_one_row_at_1000x700() {
    let i18n = I18n::english();
    let setup = compare_state(Through::Empty);
    let page = page_sized(egui::vec2(1000.0, 700.0), two_connections(), setup);
    let key = page
        .harness
        .get_by_role_and_label(Role::TextInput, &i18n.t("compare.setup.key"))
        .rect();
    let filters: Vec<Rect> = page
        .harness
        .query_all_by_role_and_label(Role::TextInput, &i18n.t("compare.setup.filter"))
        .map(|node| node.rect())
        .collect();
    assert_eq!(filters.len(), 2, "one filter per side");
    // Side by side: each starts right of the one before and all end inside the window.
    assert!(
        key.right() < filters[0].left() && filters[0].right() < filters[1].left(),
        "the key {key:?} and the filters {filters:?} are not in one row"
    );
    assert!(
        filters[1].right() <= 1000.0,
        "{filters:?} run out of the window"
    );
    // ... and level: the boxes share one top, as they did before the columns could wrap.
    for filter in &filters {
        assert!(
            (filter.top() - key.top()).abs() < 1.0,
            "the key {key:?} and the filter {filter:?} are not level"
        );
    }
}

fn two_connections() -> Vec<ConnectionConfig> {
    vec![
        ConnectionConfig::new("Production replica", "mongodb://db.example.com:27017"),
        ConnectionConfig::new("Staging", "mongodb://staging.example.com:27017"),
    ]
}

/// The audit of a page with the tab strip: the tabs scrolled out of the strip (their unclipped
/// rects lie outside the list) are not on screen and are left out, the rest of the page is audited
/// as ever. At least three tabs and the active one must be in view.
fn audit_tabs(name: &str, page: &Page) {
    let list = page
        .harness
        .get_by_role_and_label(Role::TabList, &I18n::english().t("tabs.strip"))
        .rect();
    let in_view =
        |rect: &Rect| rect.left() >= list.left() - 1.0 && rect.right() <= list.right() + 1.0;
    let shown: Vec<_> = controls(page)
        .into_iter()
        .filter(|(label, rect)| {
            let scrolled = label.starts_with("Tab ") || label.starts_with("Button Close ");
            !scrolled || in_view(rect)
        })
        .collect();
    let tabs = shown.iter().filter(|(l, _)| l.starts_with("Tab ")).count();
    assert!(
        tabs >= 3,
        "{name}: only {tabs} tabs are in view of the strip {list:?}"
    );
    audit_controls(name, shown);
}

fn six_tab_page() -> Page {
    page(vec![], six_tabs)
}

#[test]
fn browse_with_six_tabs() {
    audit_tabs("six tabs", &six_tab_page());
}

#[test]
fn browse_with_six_tabs_and_the_all_tabs_menu_open() {
    let behind = controls(&six_tab_page());
    let mut page = six_tab_page();
    page.harness
        .get_by_role_and_label(Role::Button, &I18n::english().t("tabs.all_tabs"))
        .click();
    page.harness.run_steps(4);
    // The menu floats over the page: audit its own entries, as a dialog's buttons are.
    let menu: Vec<_> = without_page_controls(controls(&page), &behind)
        .into_iter()
        .filter(|(label, _)| label.starts_with("Button "))
        .collect();
    assert_eq!(menu.len(), 6, "one entry per tab: {menu:?}");
    audit_controls("six tabs, all-tabs menu", menu);
}

/// A split view at the smallest window has no room for two panes: one pane under a chip, held to
/// the same rules as any page (the chip's button is a 24 pt target, nothing overlaps).
#[test]
fn browse_split_falls_back_to_one_pane_with_a_chip() {
    let page = page(vec![], |state| split_two(state, Orientation::SideBySide));
    audit("split, one pane and a chip", &page);
    page.harness.get_by_role_and_label(
        Role::Button,
        &I18n::english().t("tabs.split.single_show_other"),
    );
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
fn queries_with_a_field_order_row() {
    audit(
        "queries_field_order",
        &page(vec![], queries_view_field_order),
    );
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
        dialog_fits!($name, $base, $setup, vec![]);
    };
    ($name:ident, $base:expr, $setup:expr, $connections:expr) => {
        #[test]
        fn $name() {
            audit_dialog(stringify!($name), $base, &page($connections, $setup));
        }
    };
}

mod dialogs {
    use super::*;
    use support::{bulk_shots, code_export_shots, copy_shots, production_shots};

    use bson::Document;
    use curlybrace_ui::viewmodel::{EditorState, MenuItem, ViewMode, page_key};
    use support::compare_apply_states::shots as apply;
    use support::compare_export_shots as export;

    fn tree(state: &mut AppState) {
        browsing(ViewMode::Tree)(state);
    }

    fn editor_modal(state: &mut AppState) {
        tree(state);
        let first: Document = state.page.clone().expect("page installed").docs[0].clone();
        state.editor = Some(EditorState::for_edit(&first));
    }

    fn delete_confirmation(state: &mut AppState) {
        tree(state);
        let page = state.page.clone().expect("page installed");
        let key = page_key(&page);
        let (next, _) =
            std::mem::take(&mut state.tree).menu_chosen(MenuItem::Delete, &page.docs[1], &key);
        state.tree = next;
    }

    dialog_fits!(
        compare_apply_review,
        apply::pending_panel,
        apply::review_with_delete_production,
        two_connections()
    );
    dialog_fits!(
        compare_export_plan,
        export::pending_panel_with_export_button,
        export::export_plan_modal,
        two_connections()
    );
    dialog_fits!(
        compare_export_report,
        export::pending_panel_with_export_button,
        export::export_report_modal,
        two_connections()
    );
    dialog_fits!(
        aggregate_run,
        aggregate_tab,
        support::aggregate_shots::aggregate_run_dialog_tab
    );
    dialog_fits!(saved_copies, browse_table, bulk_shots::saved_copies_list);
    dialog_fits!(editor, tree, editor_modal);
    dialog_fits!(delete_confirm, tree, delete_confirmation);

    #[test]
    fn host_key_prompt() {
        let (request, _waiter) =
            HostKeyRequest::new("bastion.example.com", "SHA256:Zk3b0yQwXn8oVd");
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(request).expect("queue the question");
        let page = page_full(SMALL, two_connections(), Some(rx), |_| {});
        let base = super::page(two_connections(), |_| {});
        audit_dialog_against("host_key_prompt", &controls(&base), &page);
    }

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

/// The audit itself must be able to fail: each assertion is shown on made-up controls.
mod audit_can_fail {
    use super::*;

    fn at(label: &str, top: f32, height: f32) -> (String, Rect) {
        (
            label.to_owned(),
            Rect::from_min_size(egui::pos2(300.0, top), egui::vec2(80.0, height)),
        )
    }

    #[test]
    fn well_placed_controls_pass() {
        audit_controls("ok", vec![at("Button Run", 10.0, 28.0)]);
    }

    #[test]
    #[should_panic(expected = "no control is on screen")]
    fn nothing_on_screen_fails() {
        audit_controls("none", vec![at("Button Run", 900.0, 28.0)]);
    }

    #[test]
    #[should_panic(expected = "squashed")]
    fn a_short_button_fails() {
        audit_controls("short", vec![at("Button Run", 10.0, 20.0)]);
    }

    #[test]
    #[should_panic(expected = "first row ends below the window")]
    fn a_first_row_below_the_window_fails() {
        audit_controls("low", vec![at("Button Run", 480.0, 28.0)]);
    }

    #[test]
    fn a_dialog_button_named_like_a_page_button_is_kept() {
        let page = vec![at("Button Copy", 100.0, 28.0)];
        let shown = vec![
            at("Button Copy", 105.0, 28.0),
            at("Button Copy", 480.0, 28.0),
        ];
        let own = without_page_controls(shown, &page);
        assert_eq!(own, vec![at("Button Copy", 480.0, 28.0)]);
    }
}
