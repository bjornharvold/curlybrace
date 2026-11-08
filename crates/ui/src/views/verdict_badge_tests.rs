use super::*;
use crate::app::VerdictState;
use crate::theme::{self, Mode, Tone as ThemeTone, scheme};
use crate::viewmodel::verdict::badge;
use curlybrace_core::explain::{Verdict, VerdictKind};
use curlybrace_core::store::Query;
use egui_kittest::Harness;

const KINDS: [(VerdictKind, ThemeTone); 7] = [
    (VerdictKind::IndexUsed, ThemeTone::Good),
    (VerdictKind::CoveredByIndex, ThemeTone::Good),
    (VerdictKind::InefficientIndex, ThemeTone::Warning),
    (VerdictKind::SortNotCovered, ThemeTone::Warning),
    (VerdictKind::CollectionScan, ThemeTone::Bad),
    (VerdictKind::NoFilter, ThemeTone::Neutral),
    (VerdictKind::EmptyCollection, ThemeTone::Neutral),
];

fn verdict(kind: VerdictKind) -> Verdict {
    Verdict {
        kind,
        index_name: None,
        keys_examined: 0,
        docs_examined: 0,
        returned: 0,
        plan_only: false,
        reasons: Vec::new(),
        suggestion: None,
        namespace: None,
    }
}

fn model(kind: VerdictKind) -> BadgeModel {
    let state = VerdictState::Ready {
        query: Query::default(),
        verdict: verdict(kind),
    };
    badge(&state, None).expect("a ready verdict has a badge")
}

/// The first frame's shapes of the verdict row for `model` drawn in `mode`.
fn painted(model: BadgeModel, mode: Mode) -> Vec<egui::epaint::ClippedShape> {
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 100.0))
        .build_ui(move |ui| {
            if !installed {
                theme::install(ui.ctx());
                ui.ctx().set_theme(match mode {
                    Mode::Light => egui::Theme::Light,
                    Mode::Dark => egui::Theme::Dark,
                });
                installed = true;
            }
            show_verdict_row(ui, Some(&model), &I18n::english());
        });
    harness.run();
    harness.output().shapes.clone()
}

fn has_fill(shapes: &[egui::epaint::ClippedShape], fill: egui::Color32) -> bool {
    shapes
        .iter()
        .any(|c| matches!(&c.shape, egui::Shape::Rect(r) if r.fill == fill))
}

fn has_text_color(shapes: &[egui::epaint::ClippedShape], color: egui::Color32) -> bool {
    shapes
        .iter()
        .any(|c| matches!(&c.shape, egui::Shape::Text(t) if t.fallback_color == color))
}

#[test]
fn verdict_chip_uses_the_tone_container() {
    for mode in [Mode::Light, Mode::Dark] {
        for (kind, tone) in KINDS {
            let model = model(kind);
            assert_eq!(chip_tone(model.tone), tone, "{kind:?}");
            let colors = scheme(mode).tone(tone);
            let shapes = painted(model, mode);
            assert!(
                has_fill(&shapes, colors.container),
                "{mode:?} {kind:?} container"
            );
            assert!(
                has_text_color(&shapes, colors.on_container),
                "{mode:?} {kind:?} text"
            );
        }
    }
}

#[test]
fn the_non_verdict_states_read_as_neutral() {
    for tone in [Tone::Neutral, Tone::Unavailable, Tone::Checking] {
        assert_eq!(chip_tone(tone), ThemeTone::Neutral);
    }
}

/// (height the row took, Why button height if there is one) for `model`.
fn row_metrics(model: Option<BadgeModel>) -> (f32, Option<f32>) {
    use egui_kittest::kittest::Queryable;
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 200.0))
        .build_ui(move |ui| {
            if !installed {
                theme::install(ui.ctx());
                ui.ctx().set_theme(egui::Theme::Light);
                installed = true;
            }
            let top = ui.cursor().top();
            show_verdict_row(ui, model.as_ref(), &I18n::english());
            let height = ui.cursor().top() - top;
            ui.ctx()
                .data_mut(|d| d.insert_temp(egui::Id::new("row_height"), height));
        });
    harness.run();
    let height = harness
        .ctx
        .data(|d| d.get_temp::<f32>(egui::Id::new("row_height")))
        .expect("the row ran");
    let why = harness
        .query_by_label(&I18n::english().t("query.verdict.why"))
        .map(|n| n.rect().height());
    (height, why)
}

#[test]
fn the_why_button_is_a_theme_button_and_the_row_height_does_not_depend_on_it() {
    let mut with_details = model(VerdictKind::CollectionScan);
    with_details.plan_only = true;
    let plain = model(VerdictKind::IndexUsed);
    assert!(!plain.plan_only && plain.reasons.is_empty() && plain.suggestion.is_none());
    let (tall, why) = row_metrics(Some(with_details));
    let (short, none) = row_metrics(Some(plain));
    let (empty, _) = row_metrics(None);
    assert_eq!(none, None);
    assert_eq!(
        why,
        Some(theme::CONTROL_HEIGHT),
        "Why is a 40 dp theme button, not egui's"
    );
    assert!(
        (tall - short).abs() < 0.5 && (tall - empty).abs() < 0.5,
        "{tall} {short} {empty}"
    );
}
