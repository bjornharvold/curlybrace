//! The Browse page with the Query options section open, for the screenshots and the layout tests:
//! a page found with a collation (`en`, strength 2), the hint `status_1`, a time limit of 5000 ms
//! and a read preference of `secondaryPreferred`, the section open on that form, and a comment of
//! only spaces typed since (so one problem shows, under that field, and Find is off for it).
use super::shots::browsing;
use curlybrace_core::pipeline::{
    CollationOption, HintOption, PipelineOptions, ReadPreferenceMode, Strength,
};
use curlybrace_core::store::Query;
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::viewmodel::aggregate_hint_list::HintListUi;
use curlybrace_ui::viewmodel::aggregate_options::OptionsEdit;
use curlybrace_ui::viewmodel::{FindBarState, ViewMode};

/// The options the shown page was found with.
pub fn page_options() -> PipelineOptions {
    PipelineOptions {
        collation: Some(CollationOption {
            locale: "en".into(),
            strength: Some(Strength::Secondary),
            case_level: None,
            case_first: None,
            numeric_ordering: None,
            alternate: None,
        }),
        hint: Some(HintOption::Name("status_1".into())),
        max_time_ms: Some(5000),
        read_preference: Some(ReadPreferenceMode::SecondaryPreferred),
        ..PipelineOptions::default()
    }
}

/// The edits of the form, in the order the person made them, ending with the problem.
fn edits() -> Vec<OptionsEdit> {
    vec![
        OptionsEdit::CollationOn(true),
        OptionsEdit::Locale("en".into()),
        OptionsEdit::Strength(Some(Strength::Secondary)),
        OptionsEdit::HintText("status_1".into()),
        OptionsEdit::MaxTime("5000".into()),
        OptionsEdit::ReadPreference(Some(ReadPreferenceMode::SecondaryPreferred)),
        OptionsEdit::Comment("   ".into()),
    ]
}

/// The form of [`page_options`] with the problem, in an open section under an open More row.
pub fn find_bar() -> FindBarState {
    let state = FindBarState {
        more: Some(true),
        options_open: Some(true),
        ..FindBarState::default()
    };
    edits()
        .into_iter()
        .fold(state, |state, edit| state.options_edited(edit))
}

/// The browser of `browsing` showing a page found with [`page_options`].
pub fn find_options_page(state: &mut AppState) {
    browsing(ViewMode::Table)(state);
    state.find = find_bar();
    state.aggregate.options_ui.hint =
        HintListUi::default().from_names(vec!["_id_".into(), "city_1".into(), "status_1".into()]);
    let query = Query {
        options: page_options(),
        ..Query::default()
    };
    state.issued_query = query.clone();
    state.page_query = query;
}
