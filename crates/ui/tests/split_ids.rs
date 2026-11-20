//! The split view's egui ids (#159, slice 3): every widget of a pane is under that pane's own id,
//! so two panes on the same sub-tab never share one. `no_egui_id_clash_in_a_split_frame` fails on
//! egui's own "ID clash" warning in a split frame, on every sub-tab, with the panes on one
//! connection and on two, and with the find options open (their fields have fixed ids). Fast,
//! offline, no GPU.
mod support;

use curlybrace_ui::app::reducer::Input;
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::app::tab_actions::TabAction;
use curlybrace_ui::viewmodel::split_layout::Orientation;
use curlybrace_ui::viewmodel::{CollectionTab, EditorState};
use eframe::egui::{self, accesskit, accesskit::Role};
use egui_kittest::kittest::{NodeT, Queryable};
use support::browse_page::{BrowsePage, browse_page};
use support::tabs_shots::{split_two, split_two_connections};

/// Wide and tall enough for two panes side by side.
const WIDE: (f32, f32) = (1400.0, 900.0);
/// The standard window: the panes are stacked when asked to be.
const STANDARD: (f32, f32) = (1000.0, 700.0);

fn t(key: &str) -> &'static str {
    Box::leak(support::browse_page::t(key).into_boxed_str())
}

/// The id of the document editor's text box on screen.
fn editor_text_id(page: &BrowsePage) -> accesskit::NodeId {
    page.harness
        .get_by_role_and_label(Role::MultilineTextInput, t("editor.title_new"))
        .accesskit_node()
        .id()
}

#[test]
fn the_editor_of_each_tab_has_its_own_widget_ids() {
    // Two tabs with an editor each: their text boxes must not share egui's memory (cursor,
    // selection, scroll), or one tab's editor would open where the other's was left.
    let mut page = browse_page(WIDE, |state| {
        split_two(state, Orientation::SideBySide);
        let partner = state.tabs.split.as_ref().expect("a split").secondary;
        state.editor = Some(EditorState::for_new());
        if let Some(view) = state.parked_tabs.get_mut(&partner) {
            view.editor = Some(EditorState::for_new());
        }
    });
    page.harness.run_steps(3);
    let first = editor_text_id(&page);
    let partner = page
        .harness
        .state()
        .state()
        .tabs
        .split
        .as_ref()
        .expect("a split")
        .secondary;
    page.harness
        .state_mut()
        .dispatch(Input::Tabs(TabAction::Activate(partner)));
    page.harness.run_steps(3);
    assert_ne!(
        first,
        editor_text_id(&page),
        "the two editors share widget ids"
    );
}

/// egui paints a red "ID clash" warning into the frame's debug layer when two widgets share an
/// id at different places (it only checks while `warn_on_id_clash` is on). The warning is text
/// that starts with a flame, so it is found in the frame's shapes.
fn clash_warnings(page: &BrowsePage) -> Vec<String> {
    fn texts(shape: &egui::Shape, found: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => found.push(text.galley.text().to_string()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| texts(s, found)),
            _ => {}
        }
    }
    let mut found = Vec::new();
    for clipped in &page.harness.output().shapes {
        texts(&clipped.shape, &mut found);
    }
    found
        .into_iter()
        .filter(|t| t.contains('\u{1F525}'))
        .collect()
}

/// Puts both panes on `sub` with the find options open and an Aggregate stage in each draft,
/// wherever each tab lives (the partner is parked in the strip, or is the current tab of another
/// connection's parked view).
fn both_panes_on(state: &mut AppState, sub: CollectionTab) {
    let partner = state.tabs.split.as_ref().expect("a split").secondary;
    let shape = |find: &mut curlybrace_ui::viewmodel::FindBarState| {
        find.more = Some(true);
        find.options_open = Some(true);
    };
    state.tab = sub;
    shape(&mut state.find);
    state.aggregate = Box::new(state.aggregate.as_ref().clone().add_stage("$match"));
    if let Some(view) = state.parked_tabs.get_mut(&partner) {
        shape(&mut view.find);
        view.tab = sub;
        view.aggregate = Box::new(view.aggregate.as_ref().clone().add_stage("$match"));
        return;
    }
    let session = state.tabs.owner.get(&partner).copied().expect("an owner");
    if let Some(view) = state.parked.get_mut(&session) {
        shape(&mut view.find);
        view.tab = sub;
        view.aggregate = Box::new(view.aggregate.as_ref().clone().add_stage("$match"));
    }
}

fn no_clash_in(size: (f32, f32), setup: impl FnOnce(&mut AppState), sub: CollectionTab) {
    let mut page = browse_page(size, |state| {
        setup(state);
        both_panes_on(state, sub);
    });
    page.harness.ctx.options_mut(|o| o.warn_on_id_clash = true);
    page.harness.run_steps(4);
    assert_eq!(
        page.harness.query_all_by_role(Role::Slider).count(),
        1,
        "a split frame"
    );
    let warnings = clash_warnings(&page);
    assert!(
        warnings.is_empty(),
        "egui reports an id clash on {sub:?}: {warnings:?}"
    );
}

#[test]
fn a_global_dialog_is_drawn_once_in_a_split_frame() {
    // The Copy as code dialog belongs to the app: with two panes it is still one modal. (egui
    // does not flag two areas of the same rectangle, so the modals themselves are recorded.)
    for (size, orientation) in [
        (WIDE, Orientation::SideBySide),
        (STANDARD, Orientation::Stacked),
    ] {
        let mut page = browse_page(size, |s| split_two(s, orientation));
        page.harness.state_mut().dispatch(Input::Find(
            curlybrace_ui::viewmodel::FindAction::CopyAsCode,
        ));
        page.harness.run_steps(4);
        let twice = curlybrace_ui::views::pane_scope::modals_drawn_twice(&page.harness.ctx);
        assert!(
            twice.is_empty(),
            "a modal drawn twice in a frame: {twice:?}"
        );
    }
}

#[test]
fn no_egui_id_clash_in_a_split_frame() {
    // Two panes on one connection, and on two, side by side and stacked, on every sub-tab.
    for sub in [
        CollectionTab::Documents,
        CollectionTab::Indexes,
        CollectionTab::Aggregate,
    ] {
        no_clash_in(WIDE, |s| split_two(s, Orientation::SideBySide), sub);
        no_clash_in(STANDARD, |s| split_two(s, Orientation::Stacked), sub);
        no_clash_in(
            WIDE,
            |s| split_two_connections(s, Orientation::SideBySide),
            sub,
        );
    }
}
