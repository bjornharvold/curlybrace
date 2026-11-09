//! The Compare screens use the Material 3 widgets: step, progress and result states are tonal
//! chips whose accessible text is unchanged, the actions are the theme's buttons, and the apply
//! review is a themed dialog that still confirms the production write in words and puts Cancel
//! first. Offline and GPU-free: the real app on an in-memory store, queried by shapes and labels.
mod support;

use curlybrace_core::connection::ConnectionConfig;
use curlybrace_ui::I18n;
use curlybrace_ui::app::CurlybraceApp;
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::theme::{self, Mode, Tone, scheme, shape};
use eframe::egui::{self, Color32, Rect, Shape};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use support::compare_apply_states::shots as apply;
use support::compare_multi_states::overview_mixed;
use support::compare_states::{Through, compare_state};
use support::shots::build_app;

const WINDOW: egui::Vec2 = egui::vec2(1000.0, 2000.0);

struct Screen {
    harness: Harness<'static, CurlybraceApp>,
    _rt: tokio::runtime::Runtime,
    _dir: tempfile::TempDir,
}

fn connections() -> Vec<ConnectionConfig> {
    vec![
        ConnectionConfig::new("Production replica", "mongodb://db.example.com:27017"),
        ConnectionConfig::new("Staging", "mongodb://staging.example.com:27017"),
    ]
}

fn screen(setup: impl FnOnce(&mut AppState)) -> Screen {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .build()
        .expect("tokio runtime");
    let _enter = rt.enter();
    let (mut app, dir) = build_app(connections(), None);
    setup(app.state_mut());
    let mut installed = false;
    let mut harness = Harness::builder().with_size(WINDOW).build_state(
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
    Screen {
        harness,
        _rt: rt,
        _dir: dir,
    }
}

fn t(key: &str) -> String {
    I18n::english().t(key)
}

/// Filled rectangles of the last frame: (rect, fill, corner radius). A frame with a shadow is
/// a `Shape::Vec`, so nested shapes are flattened.
fn fills(screen: &Screen) -> Vec<(Rect, Color32, u8)> {
    fn collect(shape: &Shape, out: &mut Vec<(Rect, Color32, u8)>) {
        match shape {
            Shape::Rect(r) => out.push((r.rect, r.fill, r.corner_radius.nw)),
            Shape::Vec(inner) => inner.iter().for_each(|s| collect(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in &screen.harness.output().shapes {
        collect(&clipped.shape, &mut out);
    }
    out
}

/// Whether the node labelled with `text` (a substring) sits on a pill filled with `fill`.
fn on_pill(screen: &Screen, text: &str, fill: Color32) -> bool {
    let pills = fills(screen);
    screen
        .harness
        .query_all_by_label_contains(text)
        .map(|node| node.rect())
        .any(|label| {
            pills.iter().any(|(rect, color, radius)| {
                *color == fill && *radius >= 12 && rect.contains_rect(label)
            })
        })
}

fn tone(tone: Tone) -> Color32 {
    scheme(Mode::Light).tone(tone).container
}

fn glyph(icon: &str, key: &str) -> String {
    format!("{icon} {}", t(key))
}

#[test]
fn the_run_overview_states_are_chips() {
    let s = screen(overview_mixed);
    for (text, expected) in [
        (glyph("✔", "compare.overview.status.completed"), Tone::Good),
        (glyph("✖", "compare.overview.status.failed"), Tone::Bad),
        (glyph("▶", "compare.overview.status.running"), Tone::Neutral),
    ] {
        assert!(on_pill(&s, &text, tone(expected)), "{text} is a chip");
    }
}

#[test]
fn a_blocked_preflight_verdict_is_a_chip() {
    let s = screen(compare_state(Through::Blocked));
    assert!(
        on_pill(&s, "Needs an index", tone(Tone::Bad)),
        "the verdict is a chip"
    );
}

#[test]
fn the_finished_apply_summary_is_made_of_chips() {
    let s = screen(apply::apply_finished_mixed);
    let applied = t("compare.apply.outcome.applied").replace("{count}", "");
    assert!(on_pill(&s, applied.trim(), tone(Tone::Good)));
    let failed = t("compare.apply.outcome.failed").replace("{count}", "");
    assert!(on_pill(&s, failed.trim(), tone(Tone::Bad)));
}

#[test]
fn actions_use_the_theme_buttons() {
    let s = screen(compare_state(Through::Blocked));
    let secondary = scheme(Mode::Light).secondary_container;
    assert!(
        on_pill(&s, &t("compare.preflight.check"), secondary),
        "Check is tonal"
    );
}

#[test]
fn the_production_review_is_a_themed_dialog_that_still_warns_in_words() {
    let s = screen(apply::review_with_delete_production);
    let high = scheme(Mode::Light).surface_container_high;
    let dialog = fills(&s)
        .into_iter()
        .any(|(_, fill, radius)| fill == high && radius == shape::EXTRA_LARGE as u8);
    assert!(dialog, "the review sits on the modal frame");
    s.harness.get_by_label(&t("write_warning.title"));
    s.harness.get_by_label(&t("compare.apply.review.cancel"));
    s.harness.get_by_label(&t("compare.apply.review.apply"));
    s.harness
        .get_by_label_contains("is a production connection");
}

#[test]
fn the_production_apply_keeps_its_warning_and_focuses_cancel_first() {
    use eframe::egui::accesskit::Role;
    let s = screen(apply::review_with_delete_production);
    let (cancel_label, apply_label) = (
        t("compare.apply.review.cancel"),
        t("compare.apply.review.apply"),
    );
    let cancel = s.harness.get_by_role_and_label(Role::Button, &cancel_label);
    let apply = s.harness.get_by_role_and_label(Role::Button, &apply_label);
    assert!(
        cancel.rect().left() < apply.rect().left(),
        "Cancel is first"
    );
    assert!(
        cancel.accesskit_node().is_focused(),
        "Cancel holds the focus"
    );
    assert!(!apply.accesskit_node().is_focused());
    s.harness.get_by_label(&t("write_warning.title"));
    s.harness.get_by_label(&t("connection.production_badge"));
}
