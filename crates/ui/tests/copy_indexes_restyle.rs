//! The Copy and Indexes screens use the Material 3 widgets: index rows are pills that elide a
//! long name instead of running into the usage count, the toolbar and dialog actions are the
//! theme's buttons, the builder's key and option fields are outlined fields (with a reserved
//! row for their error), the Queries rows show their verdict as a chip, and every dialog sits on
//! the themed modal surface. Offline and GPU-free; queried by shapes and labels.
use bson::Document;
use curlybrace_core::copy::policy::ConflictPolicy;
use curlybrace_core::copy::protocol::{CopyItem, CopyRequest, CopyScope};
use curlybrace_core::explain::VerdictKind;
use curlybrace_core::history::{
    Bucket, ChartRange, Sample, ShapeBucket, ShapeSource, ShapeSummary,
};
use curlybrace_core::indexes::KeyKind;
use curlybrace_core::runtime::IndexSeries;
use curlybrace_core::shape::from_query;
use curlybrace_core::store::{CollectionIndex, Namespace, ProfilingState, Query};
use curlybrace_core::test_support::mock_objects::mock_index_spec_named;
use curlybrace_ui::I18n;
use curlybrace_ui::app::copy_state::CopyDialog;
use curlybrace_ui::app::sampling_settings::SamplingSettings;
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::theme::{self, Mode, Tone, scheme, shape};
use curlybrace_ui::viewmodel::index_builder::BUILD_WARNING_THRESHOLD;
use curlybrace_ui::viewmodel::index_detail::anchored_now;
use curlybrace_ui::viewmodel::{
    ConnectionBadge, DropConfirm, IndexBuilderState, IndexesAction, IndexesState, ProfilerConsent,
    ProfilingView, QueriesAction, QueriesState,
};
use curlybrace_ui::views::copy_dialog::show_copy_modal;
use curlybrace_ui::views::index_builder::{BuilderContext, show_index_builder};
use curlybrace_ui::views::index_dialogs::{DialogContext, show_drop_confirm};
use curlybrace_ui::views::indexes::show_indexes;
use curlybrace_ui::views::queries::show_queries;
use curlybrace_ui::views::queries_consent::show_profiler_consent;
use eframe::egui::accesskit::Role;
use eframe::egui::{self, Color32, Rect, Shape};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use uuid::Uuid;

const NOW: i64 = 1_800_000_000;
const LONG: &str = "customerId_1_createdAt_-1_lastSeenAt_1_billingCountry_1_extra_long_name";

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn light() -> theme::Scheme {
    scheme(Mode::Light).clone()
}

fn tone(tone: Tone) -> Color32 {
    light().tone(tone).container
}

/// Installs the light theme once, before the first frame.
fn themed(ctx: &egui::Context, installed: &mut bool) {
    if !*installed {
        theme::install(ctx);
        ctx.set_theme(egui::Theme::Light);
        *installed = true;
    }
}

/// Filled rectangles of the last frame: (rect, fill, corner radius).
fn fills<S>(harness: &Harness<'_, S>) -> Vec<(Rect, Color32, u8)> {
    fn collect(shape: &Shape, out: &mut Vec<(Rect, Color32, u8)>) {
        match shape {
            Shape::Rect(r) => out.push((r.rect, r.fill, r.corner_radius.nw)),
            Shape::Vec(inner) => inner.iter().for_each(|s| collect(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in &harness.output().shapes {
        collect(&clipped.shape, &mut out);
    }
    out
}

/// Whether a node whose label contains `text` sits inside a rectangle filled with `fill` whose
/// corners are rounded by at least `min_radius`.
fn on_fill<S>(harness: &Harness<'_, S>, text: &str, fill: Color32, min_radius: u8) -> bool {
    let rects = fills(harness);
    harness
        .query_all_by_label_contains(text)
        .map(|node| node.rect())
        .any(|label| {
            rects
                .iter()
                .any(|(r, c, radius)| *c == fill && *radius >= min_radius && r.contains_rect(label))
        })
}

fn spec(name: &str) -> curlybrace_core::indexes::IndexSpec {
    mock_index_spec_named(name, &[("a", KeyKind::Asc)])
}

fn series(name: &str) -> IndexSeries {
    let spec = spec(name);
    let end = anchored_now(ChartRange::Day, NOW);
    IndexSeries {
        index_name: name.into(),
        definition_hash: spec.definition_hash(),
        host: "n1".into(),
        buckets: vec![Bucket {
            start_secs: end - 600,
            width_secs: 600,
            uses: 745,
            restarts: 0,
            gap_secs: 0,
        }],
        latest: Some(Sample {
            at_secs: NOW,
            ops: 42,
            since_ms: 1,
        }),
        last_used_at_secs: Some(NOW - 600),
        tracked_since_secs: Some(end - 600),
    }
}

// ---------------------------------------------------------------- the Indexes list

fn indexes(read_only: bool) -> Harness<'static, (IndexesState, Vec<IndexesAction>)> {
    let state = IndexesState {
        ns: Some(Namespace::new("shop", "orders")),
        indexes: vec![CollectionIndex {
            spec: spec(LONG),
            size_bytes: Some(4_096),
        }],
        series: vec![series(LONG)],
        selected: Some(LONG.into()),
        ..IndexesState::default()
    };
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 700.0))
        .build_state(
            move |ctx, (state, seen): &mut (IndexesState, Vec<IndexesAction>)| {
                themed(ctx, &mut installed);
                egui::CentralPanel::default().show(ctx, |ui| {
                    let action = show_indexes(
                        ui,
                        state,
                        read_only,
                        &I18n::english(),
                        NOW,
                        &SamplingSettings::default(),
                    );
                    seen.extend(action);
                });
            },
            (state, Vec::new()),
        );
    harness.run_steps(6);
    harness
}

#[test]
fn a_long_index_name_elides_instead_of_running_into_the_usage_count() {
    let h = indexes(false);
    let row = h
        .query_all_by_label(LONG)
        .find(|n| n.accesskit_node().toggled().is_some())
        .expect("the index row");
    let uses_text = I18n::english().t_with("indexes.uses_count", &[("n", "745")]);
    let uses = h.get_by_label(&uses_text);
    assert!(
        row.rect().right() <= uses.rect().left() + 0.5,
        "name {:?} runs into usage {:?}",
        row.rect(),
        uses.rect()
    );
    // The selected row is a tonal pill.
    let selected = light().secondary_container;
    assert!(
        fills(&h)
            .iter()
            .any(|(r, c, radius)| *c == selected && *radius >= 12 && r.contains_rect(row.rect())),
        "the selected index is a pill"
    );
}

#[test]
fn refresh_is_tonal_and_create_index_is_filled() {
    let h = indexes(false);
    assert!(on_fill(
        &h,
        &t("indexes.refresh"),
        light().secondary_container,
        12
    ));
    assert!(on_fill(&h, &t("indexes.create"), light().primary, 12));
}

// ---------------------------------------------------------------- the builder

struct Built {
    builder: IndexBuilderState,
}

fn builder_harness(builder: IndexBuilderState) -> Harness<'static, Built> {
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 1000.0))
        .build_state(
            move |ctx, built: &mut Built| {
                themed(ctx, &mut installed);
                let context = BuilderContext {
                    connection: "local",
                    read_only: false,
                    busy: false,
                    existing: &[],
                };
                let action = show_index_builder(ctx, &built.builder, &context, &I18n::english());
                if let Some(IndexesAction::Builder(edit)) = action {
                    built.builder = built.builder.clone().edited(edit);
                }
            },
            Built { builder },
        );
    harness.run_steps(10);
    harness
}

fn named(key: &str, n: &str) -> String {
    I18n::english().t_with(key, &[("n", n)])
}

fn one_key() -> IndexBuilderState {
    IndexBuilderState::new(Namespace::new("shop", "orders")).with_field(0, "a")
}

#[test]
fn builder_key_rows_are_outlined_fields() {
    let h = builder_harness(one_key());
    let label = named("indexes.builder.key_field_n", "1");
    h.get_by_role_and_label(Role::TextInput, &label);
    // The label floats on the border: there is no separate caption label beside the field.
    assert_eq!(
        h.query_all_by_role_and_label(Role::Label, &label).count(),
        0
    );
    let name = t("indexes.builder.name");
    h.get_by_role_and_label(Role::TextInput, &name);
    assert_eq!(h.query_all_by_role_and_label(Role::Label, &name).count(), 0);
}

#[test]
fn the_key_type_is_still_chosen_by_label_with_its_selected_state() {
    let h = builder_harness(one_key().with_kind(0, KeyKind::Desc));
    let label = named("indexes.builder.key_type_n", "1");
    let combo = h.get_by_role_and_label(Role::ComboBox, &label);
    assert_eq!(
        combo.accesskit_node().value().as_deref(),
        Some(t("indexes.builder.kind_desc").as_str())
    );
}

#[test]
fn a_ttl_error_is_in_place_and_does_not_move_the_fields_below() {
    let ttl = t("indexes.builder.ttl");
    let rect_below = |builder: IndexBuilderState| {
        let h = builder_harness(builder);
        (
            h.get_by_label(&t("indexes.builder.unique")).rect().top(),
            h.get_by_role_and_label(Role::TextInput, &ttl)
                .accesskit_node()
                .description(),
        )
    };
    let (ok_top, ok_description) = rect_below(one_key().with_ttl(Some(5)));
    let (bad_top, bad_description) = rect_below(one_key().with_ttl(Some(-5)));
    assert_eq!(ok_description, None);
    assert_eq!(
        bad_description.as_deref(),
        Some(t("indexes.builder.problem_ttl_non_negative").as_str())
    );
    assert!(
        (ok_top - bad_top).abs() < 0.5,
        "the dialog jumped: {ok_top} vs {bad_top}"
    );
}

#[test]
fn a_duplicate_field_marks_its_row() {
    let builder = one_key().adding_key().with_field(1, "a");
    let h = builder_harness(builder);
    let label = named("indexes.builder.key_field_n", "2");
    let second = h.get_by_role_and_label(Role::TextInput, &label);
    assert!(second.accesskit_node().description().is_some());
}

#[test]
fn the_warning_is_a_container_create_is_filled_and_the_preview_is_in_a_card() {
    let large = BUILD_WARNING_THRESHOLD + 1;
    let h = builder_harness(one_key().with_estimate(large));
    let warning = I18n::english().t_with(
        "indexes.builder.warning_large",
        &[("documents", &large.to_string())],
    );
    assert!(on_fill(&h, &warning, tone(Tone::Warning), 12), "warning");
    assert!(on_fill(
        &h,
        &t("indexes.builder.create"),
        light().primary,
        12
    ));
    let heading = t("indexes.builder.preview_heading");
    let preview = h.get_by_role_and_label(Role::MultilineTextInput, &heading);
    let card = light().surface_container_low;
    assert!(
        fills(&h).iter().any(|(r, c, radius)| *c == card
            && *radius >= shape::MEDIUM as u8
            && r.contains_rect(preview.rect())),
        "the preview sits in a card"
    );
}

// ---------------------------------------------------------------- the dialogs

fn modal_surface<S>(h: &Harness<'_, S>) -> bool {
    let want = light().surface_container_high;
    fills(h)
        .iter()
        .any(|(_, c, radius)| *c == want && *radius >= shape::EXTRA_LARGE as u8)
}

#[test]
fn the_drop_confirmation_is_a_themed_dialog_with_an_outlined_name_field() {
    let name = "My.Idx";
    let mut installed = false;
    let mut h = Harness::builder()
        .with_size(egui::vec2(900.0, 700.0))
        .build_state(
            move |ctx, confirm: &mut DropConfirm| {
                themed(ctx, &mut installed);
                let badge = ConnectionBadge::default();
                let context = DialogContext {
                    connection: "local",
                    badge: &badge,
                    collection: "shop.orders",
                    busy: false,
                };
                if let Some(IndexesAction::DropTyped(text)) =
                    show_drop_confirm(ctx, confirm, &context, &I18n::english())
                {
                    *confirm = confirm.clone().with_typed(text);
                }
            },
            DropConfirm::new(name.into()).with_typed(name.into()),
        );
    h.run_steps(8);
    assert!(modal_surface(&h), "modal surface");
    let caption = I18n::english().t_with("indexes.drop.type_name", &[("name", name)]);
    h.get_by_role_and_label(Role::TextInput, &caption);
    assert_eq!(
        h.query_all_by_role_and_label(Role::Label, &caption).count(),
        0
    );
    assert!(on_fill(&h, &t("indexes.drop.confirm"), light().primary, 12));
}

#[test]
fn the_consent_dialog_keeps_its_command_and_wording_on_a_themed_surface() {
    let mut installed = false;
    let mut h = Harness::builder()
        .with_size(egui::vec2(900.0, 600.0))
        .build_state(
            move |ctx, consent: &mut ProfilerConsent| {
                themed(ctx, &mut installed);
                show_profiler_consent(ctx, consent, (false, false, false), false, &I18n::english());
            },
            ProfilerConsent {
                db: "shop".into(),
                level: 2,
            },
        );
    h.run_steps(8);
    assert!(modal_surface(&h), "modal surface");
    h.get_by_label_contains("db.getSiblingDB(\"shop\").setProfilingLevel(2)");
    assert!(on_fill(
        &h,
        &t("indexes.queries.consent.confirm_on"),
        light().primary,
        12
    ));
}

// ---------------------------------------------------------------- Queries

fn shape_json(field: &str) -> String {
    from_query(&Query {
        filter: bson::doc! { field: 1 },
        ..Query::default()
    })
    .json()
}

fn summary(field: &str, worst: VerdictKind, misses: u64) -> ShapeSummary {
    ShapeSummary {
        hash: format!("hash-{field}"),
        shape_json: shape_json(field),
        source: ShapeSource::App,
        executions: 42,
        misses,
        total_millis: 900,
        max_millis: 350,
        worst,
        latest: worst,
        buckets: vec![ShapeBucket {
            start_secs: NOW - 1_200,
            width_secs: 600,
            executions: 42,
            misses,
            gap_secs: 0,
        }],
    }
}

fn queries() -> Harness<'static, QueriesState> {
    let state = QueriesState::default()
        .with_profiling(ProfilingView::Known(ProfilingState {
            level: 0,
            slow_ms: 100,
        }))
        .with_shapes(vec![
            summary("status", VerdictKind::CollectionScan, 12),
            summary("sku", VerdictKind::IndexUsed, 0),
        ]);
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 800.0))
        .build_state(
            move |ctx, state: &mut QueriesState| {
                themed(ctx, &mut installed);
                egui::CentralPanel::default().show(ctx, |ui| {
                    let ns = Namespace::new("shop", "orders");
                    let _: Option<QueriesAction> =
                        show_queries(ui, state, &ns, false, &I18n::english(), NOW);
                });
            },
            state,
        );
    harness.run_steps(6);
    harness
}

#[test]
fn query_rows_show_their_verdict_as_a_chip_with_the_same_text() {
    let h = queries();
    assert!(
        on_fill(
            &h,
            "\u{26a0} Collection scan: no index used",
            tone(Tone::Bad),
            12
        ),
        "the miss is a bad chip"
    );
    assert!(
        on_fill(&h, "\u{2714} Uses an index", tone(Tone::Good), 12),
        "the hit is a good chip"
    );
}

#[test]
fn enable_profiler_is_the_primary_button() {
    let h = queries();
    assert!(on_fill(
        &h,
        &t("indexes.queries.enable"),
        light().primary,
        12
    ));
}

// ---------------------------------------------------------------- the copy dialog

fn item(name: &str) -> CopyItem {
    CopyItem {
        source: Namespace::new("shop", name),
        target: Namespace::new("crm", name),
        filter: Document::new(),
    }
}

#[test]
fn the_copy_dialog_is_themed_with_outlined_name_fields() {
    let request = CopyRequest {
        source: Uuid::from_u128(1),
        target: Uuid::from_u128(2),
        scope: CopyScope::Collections(vec![item("orders")]),
        policy: ConflictPolicy::Skip,
        with_indexes_and_options: false,
    };
    let app = AppState {
        copy: Some(CopyDialog::opened(request)),
        ..AppState::default()
    };
    let mut installed = false;
    let mut h = Harness::builder()
        .with_size(egui::vec2(900.0, 1100.0))
        .build_state(
            move |ctx, app: &mut AppState| {
                themed(ctx, &mut installed);
                show_copy_modal(ctx, app, &I18n::english());
            },
            app,
        );
    h.run_steps(8);
    assert!(modal_surface(&h), "modal surface");
    let label = I18n::english().t_with("copy.dialog.name_label", &[("source", "orders")]);
    h.get_by_role_and_label(Role::TextInput, &label);
    assert_eq!(
        h.query_all_by_role_and_label(Role::Label, &label).count(),
        0
    );
    assert!(on_fill(&h, &t("copy.dialog.copy"), light().primary, 12));
}
