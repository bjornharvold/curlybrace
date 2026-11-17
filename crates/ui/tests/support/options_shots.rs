//! The Aggregate tab with its Options section open, for the screenshots and the layout tests:
//! the three-stage pipeline of [`super::aggregate_shots::three_stage_draft`] with a collation
//! (`en`, strength 2), the hint `status_1`, a time limit of 5000 ms, a read preference of
//! `secondaryPreferred`, then a time limit typed as "5 s" (so one problem shows, under that field,
//! while the applied limit stays 5000 ms).
use super::aggregate_shots::{grouped_docs, three_stage_draft};
use super::counts_shots::tab_of;
use curlybrace_core::pipeline::{ReadPreferenceMode, Strength};
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::viewmodel::AggregateState;
use curlybrace_ui::viewmodel::aggregate_hint_list::HintListUi;
use curlybrace_ui::viewmodel::aggregate_options::OptionsEdit;

/// The edits of the fixture, in the order the person made them.
fn edits() -> Vec<OptionsEdit> {
    vec![
        OptionsEdit::CollationOn(true),
        OptionsEdit::Locale("en".into()),
        OptionsEdit::Strength(Some(Strength::Secondary)),
        OptionsEdit::HintText("status_1".into()),
        OptionsEdit::MaxTime("5000".into()),
        OptionsEdit::ReadPreference(Some(ReadPreferenceMode::SecondaryPreferred)),
    ]
}

/// The pipeline with the options above applied and the section open; the time limit was then
/// retyped as text that is not a number, so the form has a problem and says so under the field.
pub fn options_draft() -> AggregateState {
    let applied = edits()
        .into_iter()
        .fold(three_stage_draft(), |d, e| d.options_edited(e))
        .options_applied()
        .0;
    let mut draft = applied
        .options_toggled()
        .options_edited(OptionsEdit::MaxTime("5 s".into()));
    draft.options_ui.hint =
        HintListUi::default().from_names(vec!["_id_".into(), "city_1".into(), "status_1".into()]);
    // Applying the options asked for the preview again: it is the same documents.
    let (previewing, sent) = draft.begin_preview();
    let (request, _) = sent.expect("the fixture pipeline is valid");
    previewing.arrived(request, Ok(grouped_docs()))
}

/// [`options_draft`] in the Aggregate tab.
pub fn options_tab(state: &mut AppState) {
    tab_of(options_draft())(state);
}
