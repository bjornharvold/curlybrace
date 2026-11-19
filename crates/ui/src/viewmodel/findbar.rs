//! Find bar state: raw query text plus the last validation error.
use super::findbar_more::error_concerns_more;
use curlybrace_core::error::CoreError;
use curlybrace_core::query::{QueryText, parse_query};
use curlybrace_core::store::Query;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FindBarState {
    pub text: QueryText,
    pub last_error: Option<CoreError>,
    /// The choice for the "More" disclosure (Sort, Projection, Skip, Limit); `None` follows the
    /// content (see [`super::findbar_more::more_open`]) until it is first shown open, which then
    /// sticks (see [`Self::more_settled`]). Clear drops the choice.
    pub more: Option<bool>,
    /// Whether the text parses as typed for the Copy as code button, remembered between
    /// repaints (see `code_export_memo`); keyed on the text, so it needs no reset.
    pub export_memo: super::code_export_memo::FindExportMemo,
    /// The Query options form as typed (collation, hint, time limit, read preference, comment,
    /// disk use); Find parses it once into the query's options.
    pub options: super::aggregate_options::OptionsForm,
    /// The choice for the Query options section; `None` follows the form (open while an option
    /// is set) until it is first shown open, which then sticks (see [`Self::options_settled`]).
    /// Clear drops the choice.
    pub options_open: Option<bool>,
    /// `last_error` is an answer to the options a find carried (a missing hint, a refused
    /// collation, an unsatisfiable read preference, or a timeout under the person's own limit):
    /// editing the form or finding again drops it.
    pub error_from_options: bool,
}

impl FindBarState {
    /// Same text and disclosure, no error (used by the view after a successful submit).
    pub fn cleared_error(&self) -> Self {
        Self {
            last_error: None,
            error_from_options: false,
            ..self.clone()
        }
    }
}

/// What the user asked for; the caller performs it.
#[allow(clippy::large_enum_variant)] // contract: Run carries the Query by value
#[derive(Debug, Clone, PartialEq)]
pub enum FindAction {
    Run(Query),
    Prev,
    Next,
    Clear,
    /// The "Check index use" box was toggled; the value is the new setting.
    SetAutomaticVerdict(bool),
    /// The "Warn before full scans" box was toggled; the value is the new setting.
    SetGuard(bool),
    /// The guard's notice: send the find it held back, once, without checking it again.
    GuardRunAnyway,
    /// The guard's notice: drop it and the find it held back.
    GuardCancel,
    /// Copy as code was clicked: opens the dialog for the query the Find button would send.
    CopyAsCode,
    /// The Query options section is open and wants the collection's index names for its hint
    /// picker (asked once; the Indexes tab's list is used when it holds this collection).
    HintListWanted,
    /// The person asked to read a hint list that failed again.
    HintListRetry,
    /// Create index... in the verdict popup: open the index builder with the suggestion of the
    /// verdict on screen.
    CreateSuggestedIndex,
}

/// A raw user gesture on the find bar, before validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindIntent {
    /// Enter pressed in a text box.
    Enter,
    RunButton,
    ClearButton,
    PrevButton,
    NextButton,
    /// The Copy as code button (only offered while the text parses).
    CodeButton,
}

/// Chooses the intent for a frame; a button click wins over Enter.
pub fn pick_intent(button: Option<FindIntent>, enter: bool) -> Option<FindIntent> {
    button.or(enter.then_some(FindIntent::Enter))
}

/// Maps an intent to the new bar state and the action to emit. Run/Enter emit `Run` only when
/// the text parses; Prev/Next are ignored when not available; Clear resets text and error.
pub fn resolve_find_intent(
    state: &FindBarState,
    intent: FindIntent,
    has_prev: bool,
    has_next: bool,
) -> (FindBarState, Option<FindAction>) {
    match intent {
        FindIntent::Enter | FindIntent::RunButton => match submit_find(state) {
            Ok(q) => (state.cleared_error(), Some(FindAction::Run(q))),
            Err(failed) => (failed, None),
        },
        FindIntent::ClearButton => (FindBarState::default(), Some(FindAction::Clear)),
        FindIntent::PrevButton => (state.clone(), has_prev.then_some(FindAction::Prev)),
        FindIntent::NextButton => (state.clone(), has_next.then_some(FindAction::Next)),
        // The bar's text stays as typed; the dialog reads it (and the button is off when it
        // does not parse).
        FindIntent::CodeButton => (state.clone(), Some(FindAction::CopyAsCode)),
    }
}

/// Parses the bar text and the options. On failure returns a new state carrying the error with
/// the text untouched, so the user can fix what they typed; an error about a field behind "More"
/// opens it, so the field and its message are on screen together. The text is checked first; a
/// form with a problem sends nothing and opens More and the Query options section, where each
/// problem is told under its field.
#[allow(clippy::result_large_err)] // signature mandated: Err carries the full new state
pub fn submit_find(state: &FindBarState) -> Result<Query, FindBarState> {
    let query = parse_query(&state.text).map_err(|e| {
        let more = match error_concerns_more(&state.text) {
            true => Some(true),
            false => state.more,
        };
        FindBarState {
            last_error: Some(e),
            more,
            ..state.clone()
        }
    })?;
    match state.options.parsed() {
        Ok(options) if !state.options_blocked() => Ok(Query {
            options: options.clone(),
            ..query
        }),
        _ => Err(FindBarState {
            last_error: None,
            error_from_options: false,
            more: Some(true),
            options_open: Some(true),
            ..state.clone()
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use curlybrace_core::store::PAGE_SIZE;

    fn with_filter(f: &str) -> FindBarState {
        FindBarState {
            text: QueryText {
                filter: f.into(),
                ..Default::default()
            },
            last_error: Some(CoreError::InvalidJson("old".into())),
            more: None,
            ..Default::default()
        }
    }

    #[test]
    fn bad_json_sets_error_and_preserves_text() {
        let s = with_filter("{ not json");
        let err = submit_find(&s).unwrap_err();
        assert_eq!(err.text, s.text);
        assert!(matches!(err.last_error, Some(CoreError::InvalidJson(_))));
    }

    #[test]
    fn bad_sort_is_also_reported_with_all_text_kept() {
        let mut s = with_filter("{\"a\": 1}");
        s.text.sort = "[1]".into();
        s.text.limit = "7".into();
        let err = submit_find(&s).unwrap_err();
        assert_eq!(err.text, s.text);
        assert!(err.last_error.is_some());
    }

    #[test]
    fn good_json_returns_query_and_cleared_error_drops_the_error() {
        let s = with_filter("{\"a\": 1}");
        let q = submit_find(&s).unwrap();
        assert_eq!(q.filter, bson::doc! {"a": 1});
        assert_eq!(q.limit, PAGE_SIZE);
        let cleared = s.cleared_error();
        assert_eq!(cleared.last_error, None);
        assert_eq!(cleared.text, s.text);
    }

    fn typed(filter: &str) -> FindBarState {
        FindBarState {
            text: QueryText {
                filter: filter.into(),
                sort: "  {\"a\": 1}\n".into(),
                ..Default::default()
            },
            last_error: None,
            more: None,
            ..Default::default()
        }
    }

    #[test]
    fn parse_failure_keeps_text_byte_for_byte_and_emits_nothing() {
        let s = typed("  {oops \t");
        for intent in [FindIntent::Enter, FindIntent::RunButton] {
            let (next, action) = resolve_find_intent(&s, intent, true, true);
            assert_eq!(action, None);
            assert_eq!(next.text, s.text);
            assert!(matches!(next.last_error, Some(CoreError::InvalidJson(_))));
        }
    }

    #[test]
    fn success_runs_the_query_and_clears_the_error() {
        let mut s = typed("{\"a\": 1}");
        s.last_error = Some(CoreError::InvalidJson("old".into()));
        let (next, action) = resolve_find_intent(&s, FindIntent::RunButton, false, false);
        assert!(matches!(action, Some(FindAction::Run(q)) if q.filter == bson::doc! {"a": 1}));
        assert_eq!(next.last_error, None);
        assert_eq!(next.text, s.text);
    }

    #[test]
    fn paging_is_gated_by_availability() {
        let s = typed("{");
        let (kept, a) = resolve_find_intent(&s, FindIntent::PrevButton, false, true);
        assert_eq!((a, &kept), (None, &s));
        let p = resolve_find_intent(&s, FindIntent::PrevButton, true, false);
        assert_eq!(p, (s.clone(), Some(FindAction::Prev)));
        let n = resolve_find_intent(&s, FindIntent::NextButton, true, false);
        assert_eq!(n, (s.clone(), None));
        let n = resolve_find_intent(&s, FindIntent::NextButton, false, true);
        assert_eq!(n, (s, Some(FindAction::Next)));
    }

    #[test]
    fn clear_resets_text_and_error() {
        let mut s = typed("{oops");
        s.last_error = Some(CoreError::InvalidJson("x".into()));
        let (next, action) = resolve_find_intent(&s, FindIntent::ClearButton, false, false);
        assert_eq!(next, FindBarState::default());
        assert_eq!(action, Some(FindAction::Clear));
    }

    #[test]
    fn button_wins_over_enter_in_the_same_frame() {
        assert_eq!(
            pick_intent(Some(FindIntent::ClearButton), true),
            Some(FindIntent::ClearButton)
        );
        assert_eq!(pick_intent(None, true), Some(FindIntent::Enter));
        assert_eq!(pick_intent(None, false), None);
    }
}
