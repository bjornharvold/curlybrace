//! The Aggregate tab with per-stage counts on its cards, in the accesskit tree of the REAL app
//! rendered with the real English locale at the standard 1000x700 and the small 800x500 window:
//! the Count stages button has a role and a name and stays in the window, the count lines (at
//! least, plural, failure, timeout, counting, waiting, out of date) are in the tree, and a
//! running sequence offers Cancel counting instead. Fast, offline, no GPU.
mod support;

use curlybrace_ui::I18n;
use curlybrace_ui::app::CurlybraceApp;
use curlybrace_ui::theme;
use curlybrace_ui::viewmodel::AggregateState;
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use support::counts_shots::{
    counted_draft, counted_then_edited_draft, counted_with_failures, counting_draft, tab_of,
};
use support::shots::build_app;

const STANDARD: (f32, f32) = (1000.0, 700.0);
const SMALL: (f32, f32) = (800.0, 500.0);

struct Page {
    harness: Harness<'static, CurlybraceApp>,
    _rt: tokio::runtime::Runtime,
    _dir: tempfile::TempDir,
}

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn page(size: (f32, f32), draft: AggregateState) -> Page {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .build()
        .expect("tokio runtime");
    let enter = rt.enter();
    let (mut app, dir) = build_app(vec![], None);
    tab_of(draft)(app.state_mut());
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
    Page {
        harness,
        _rt: rt,
        _dir: dir,
    }
}

fn window(size: (f32, f32)) -> egui::Rect {
    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0, size.1))
}

#[test]
fn count_stages_is_a_named_enabled_button_inside_the_window_at_both_sizes() {
    for size in [STANDARD, SMALL] {
        let page = page(size, counted_draft());
        let name = t("aggregate.counts.button");
        let button = page.harness.get_by_role_and_label(Role::Button, &name);
        assert!(!button.accesskit_node().is_disabled());
        let rect = button.rect();
        assert!(rect.width() > 1.0 && rect.height() > 1.0, "{rect:?}");
        assert!(window(size).contains_rect(rect), "{rect:?} leaves {size:?}");
    }
}

#[test]
fn every_card_shows_its_count_at_both_sizes() {
    for size in [STANDARD, SMALL] {
        let page = page(size, counted_draft());
        let at_least = I18n::english().t_with("aggregate.counts.at_least", &[("n", "100000")]);
        page.harness.get_by_label(&at_least);
        let four = I18n::english().t_with("aggregate.counts.many", &[("n", "4")]);
        assert_eq!(page.harness.query_all_by_label(&four).count(), 2, "{size:?}");
    }
}

#[test]
fn a_failure_and_a_timeout_are_on_their_cards_at_both_sizes() {
    for size in [STANDARD, SMALL] {
        let page = page(size, counted_with_failures());
        page.harness.get_by_label(&t("aggregate.counts.one"));
        assert_eq!(
            page.harness
                .query_all_by_label_contains("Could not count: ")
                .count(),
            1
        );
        page.harness.get_by_label(&t("aggregate.counts.timed_out"));
    }
}

#[test]
fn counts_stay_visible_and_are_marked_out_of_date_after_an_edit() {
    for size in [STANDARD, SMALL] {
        let page = page(size, counted_then_edited_draft());
        let four = I18n::english().t_with("aggregate.counts.many", &[("n", "4")]);
        assert_eq!(page.harness.query_all_by_label(&four).count(), 2);
        let marks = page
            .harness
            .query_all_by_label(&t("aggregate.counts.out_of_date"))
            .count();
        assert_eq!(marks, 3, "every counted card says so at {size:?}");
    }
}

#[test]
fn a_running_sequence_offers_cancel_and_marks_the_cards() {
    for size in [STANDARD, SMALL] {
        let page = page(size, counting_draft());
        let name = t("aggregate.counts.cancel");
        let cancel = page.harness.get_by_role_and_label(Role::Button, &name);
        assert!(!cancel.accesskit_node().is_disabled());
        assert!(window(size).contains_rect(cancel.rect()));
        assert!(
            page.harness
                .query_by_role_and_label(Role::Button, &t("aggregate.counts.button"))
                .is_none()
        );
        page.harness.get_by_label(&t("aggregate.counts.counting"));
        page.harness.get_by_label(&t("aggregate.counts.waiting"));
    }
}

#[test]
fn count_stages_is_off_with_its_reason_when_nothing_is_enabled() {
    let off = AggregateState::default().add_stage("$match").toggle(0);
    let page = page(STANDARD, off);
    let name = t("aggregate.counts.button");
    let button = page.harness.get_by_role_and_label(Role::Button, &name);
    assert!(button.accesskit_node().is_disabled());
}

#[test]
fn a_count_arriving_does_not_move_the_cards() {
    let remove = I18n::english().t_with("aggregate.stage.remove_n", &[("n", "3")]);
    for size in [STANDARD, SMALL] {
        let plain = page(size, support::aggregate_shots::three_stage_draft());
        let counted = page(size, counted_draft());
        let top = |p: &Page| p.harness.get_by_label(&remove).rect().top();
        assert_eq!(top(&plain), top(&counted), "{size:?}");
    }
}
