//! The Sampling section of the Indexes tab in the accesskit tree: the excluded databases are
//! listed, each with a remove button that names its database, and the input and the Exclude
//! button have names. Fast, offline, no GPU.
use curlybrace_ui::I18n;
use curlybrace_ui::app::sampling_settings::SamplingSettings;
use curlybrace_ui::viewmodel::{IndexesAction, IndexesState};
use curlybrace_ui::views::indexes::show_indexes;
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

const NOW: i64 = 1_800_000_000;

fn t(key: &str) -> String {
    I18n::english().t(key)
}

type Frame = (SamplingSettings, Vec<IndexesAction>);

/// Draws the tab with `sampling`; the section is opened the way a person would.
fn open(sampling: SamplingSettings) -> Harness<'static, Frame> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 700.0))
        .build_ui_state(
            |ui, (sampling, seen): &mut Frame| {
                let state = IndexesState::default();
                let action = show_indexes(ui, &state, false, &I18n::english(), NOW, sampling);
                seen.extend(action);
            },
            (sampling, Vec::new()),
        );
    harness.run_steps(3);
    harness.get_by_label(&t("indexes.sampling.heading")).click();
    harness.run_steps(3);
    harness
}

fn two() -> SamplingSettings {
    SamplingSettings::default()
        .excluding("logs")
        .excluding("my.db")
}

#[test]
fn the_excluded_list_is_editable_from_the_tab_and_labelled() {
    let mut harness = open(two());
    // The built-in list is stated, not editable.
    harness.get_by_label(&t("indexes.sampling.built_in"));
    // The list: a heading and every excluded name.
    harness.get_by_label(&t("indexes.sampling.excluded_heading"));
    harness.get_by_label("logs");
    harness.get_by_label("my.db");
    // Each remove button says which database it removes.
    for name in ["logs", "my.db"] {
        let label = I18n::english().t_with("indexes.sampling.include_named", &[("name", name)]);
        harness.get_by_role_and_label(Role::Button, &label);
    }
    // The input has a name, and so does the button next to it.
    harness.get_by_role_and_label(Role::TextInput, &t("indexes.sampling.input"));
    let add_label = t("indexes.sampling.add");
    let add = harness.get_by_role_and_label(Role::Button, &add_label);
    assert!(add.accesskit_node().is_disabled(), "nothing typed yet");

    // Removing reports the database by its exact name.
    let label = I18n::english().t_with("indexes.sampling.include_named", &[("name", "my.db")]);
    harness.get_by_role_and_label(Role::Button, &label).click();
    harness.run_steps(2);
    assert_eq!(
        harness.state().1,
        vec![IndexesAction::IncludeDatabase("my.db".into())]
    );
}

#[test]
fn typing_a_name_and_pressing_exclude_reports_it_and_clears_the_box() {
    let mut harness = open(SamplingSettings::default());
    harness.get_by_label(&t("indexes.sampling.none"));
    let input_label = t("indexes.sampling.input");
    let input = harness.get_by_role_and_label(Role::TextInput, &input_label);
    input.focus();
    input.type_text("  my db ");
    harness.run_steps(3);
    harness
        .get_by_role_and_label(Role::Button, &t("indexes.sampling.add"))
        .click();
    harness.run_steps(3);
    // The view reports what was typed; trimming is the reducer's job.
    assert_eq!(
        harness.state().1,
        vec![IndexesAction::ExcludeDatabase("  my db ".into())]
    );
    let input = harness.get_by_role_and_label(Role::TextInput, &input_label);
    assert!(
        input
            .accesskit_node()
            .value()
            .unwrap_or_default()
            .is_empty(),
        "the box is empty again"
    );
}

#[test]
fn whitespace_alone_cannot_be_submitted() {
    let mut harness = open(SamplingSettings::default());
    let (input_label, add_label) = (t("indexes.sampling.input"), t("indexes.sampling.add"));
    let input = harness.get_by_role_and_label(Role::TextInput, &input_label);
    input.focus();
    input.type_text("   ");
    harness.run_steps(3);
    let add = harness.get_by_role_and_label(Role::Button, &add_label);
    assert!(add.accesskit_node().is_disabled());
    assert!(harness.state().1.is_empty());
}
