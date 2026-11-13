//! The query bar's "More" disclosure, which holds Sort, Projection, Skip and Limit under the
//! always-visible Filter. Pure: whether it is open, how many of its fields hold a value, and
//! whether a parse error just produced is about one of them.
use super::findbar::FindBarState;
use curlybrace_core::query::{QueryText, parse_query};

/// How many of the fields behind "More" hold something other than blanks.
pub fn hidden_count(text: &QueryText) -> usize {
    [&text.sort, &text.projection, &text.skip, &text.limit]
        .into_iter()
        .filter(|value| !value.trim().is_empty())
        .count()
}

/// Whether a parse error of `text`, produced from this very text, is about a field behind
/// "More": the Filter is parsed first, so when it parses alone the error can only come from Sort,
/// Projection, Skip or Limit. Asked once, when the error is produced (see
/// [`super::findbar::submit_find`]); a later edit must not reinterpret a stale error.
pub(super) fn error_concerns_more(text: &QueryText) -> bool {
    let filter_only = QueryText {
        filter: text.filter.clone(),
        ..QueryText::default()
    };
    parse_query(&filter_only).is_ok()
}

/// Whether the disclosure is open: the choice made (by the person, or by a failed run about a
/// field behind it), else open while a field behind it holds a value.
pub fn more_open(state: &FindBarState) -> bool {
    state.more.unwrap_or_else(|| hidden_count(&state.text) > 0)
}

impl FindBarState {
    /// The disclosure flipped from what it shows now; the choice then sticks until Clear.
    pub fn more_toggled(&self) -> Self {
        Self {
            more: Some(!more_open(self)),
            ..self.clone()
        }
    }

    /// Opening by itself is a one-time event: once the disclosure is shown open with no choice
    /// made, that becomes the choice, so deleting the last value behind it does not close it
    /// under the cursor. Clear drops the choice again.
    pub fn more_settled(&self) -> Self {
        match (self.more, more_open(self)) {
            (None, true) => Self {
                more: Some(true),
                ..self.clone()
            },
            _ => self.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viewmodel::{FindIntent, resolve_find_intent};

    fn with(text: QueryText) -> FindBarState {
        FindBarState {
            text,
            ..FindBarState::default()
        }
    }

    #[test]
    fn closed_while_every_hidden_field_is_blank() {
        let state = with(QueryText {
            filter: "{\"a\": 1}".into(),
            sort: "  ".into(),
            ..QueryText::default()
        });
        assert_eq!(hidden_count(&state.text), 0);
        assert!(!more_open(&state));
    }

    #[test]
    fn opens_by_itself_when_a_hidden_field_holds_a_value() {
        for text in [
            QueryText {
                sort: "{\"a\": 1}".into(),
                ..QueryText::default()
            },
            QueryText {
                limit: "5".into(),
                ..QueryText::default()
            },
        ] {
            assert!(more_open(&with(text)));
        }
    }

    #[test]
    fn counts_every_hidden_field_that_holds_a_value() {
        let text = QueryText {
            filter: "{}".into(),
            sort: "{\"a\": 1}".into(),
            projection: "{\"a\": 1}".into(),
            skip: "2".into(),
            limit: String::new(),
        };
        assert_eq!(hidden_count(&text), 3);
    }

    #[test]
    fn an_error_about_a_hidden_field_opens_it_but_a_filter_error_does_not() {
        let run = |text| resolve_find_intent(&with(text), FindIntent::RunButton, false, false).0;
        let sort_error = run(QueryText {
            sort: "x".into(),
            ..QueryText::default()
        });
        assert!(sort_error.last_error.is_some());
        assert!(more_open(&sort_error));
        let filter_error = run(QueryText {
            filter: "{ oops".into(),
            ..QueryText::default()
        });
        assert!(filter_error.last_error.is_some());
        assert_eq!(filter_error.more, None);
        assert!(!more_open(&filter_error));
    }

    #[test]
    fn fixing_the_filter_after_its_error_leaves_more_as_it_was() {
        let (failed, _) = resolve_find_intent(
            &with(QueryText {
                filter: "{ oops".into(),
                ..QueryText::default()
            }),
            FindIntent::RunButton,
            false,
            false,
        );
        let fixed = FindBarState {
            text: QueryText {
                filter: "{}".into(),
                ..failed.text.clone()
            },
            ..failed
        };
        assert!(
            fixed.last_error.is_some(),
            "the error stays until the next run"
        );
        assert!(
            !more_open(&fixed),
            "an edit of the Filter must not open More"
        );
    }

    #[test]
    fn settling_keeps_an_automatic_opening_after_the_last_value_goes() {
        let settled = with(QueryText {
            sort: "{\"a\": 1}".into(),
            ..QueryText::default()
        })
        .more_settled();
        assert_eq!(settled.more, Some(true));
        let emptied = FindBarState {
            text: QueryText::default(),
            ..settled
        };
        assert!(more_open(&emptied));
        assert_eq!(FindBarState::default().more_settled().more, None);
        let closed = FindBarState {
            more: Some(false),
            ..emptied
        };
        assert_eq!(closed.more_settled(), closed, "a choice made is kept");
    }

    #[test]
    fn toggling_overrides_the_automatic_state_both_ways() {
        let empty = FindBarState::default();
        let opened = empty.more_toggled();
        assert!(more_open(&opened));
        assert_eq!(opened.text, empty.text);
        let set = with(QueryText {
            skip: "4".into(),
            ..QueryText::default()
        });
        let closed = set.more_toggled();
        assert!(!more_open(&closed), "closed although a value is set");
        assert!(more_open(&closed.more_toggled()));
    }

    #[test]
    fn a_failed_run_about_a_hidden_field_reopens_a_closed_disclosure() {
        let state = with(QueryText {
            sort: "[1]".into(),
            ..QueryText::default()
        })
        .more_toggled();
        assert!(!more_open(&state));
        let (next, action) = resolve_find_intent(&state, FindIntent::RunButton, false, false);
        assert_eq!(action, None);
        assert!(more_open(&next), "the error is shown with its field");
    }

    #[test]
    fn run_and_paging_keep_the_choice_and_clear_drops_it() {
        let opened = FindBarState::default().more_toggled();
        let (ran, _) = resolve_find_intent(&opened, FindIntent::RunButton, false, false);
        assert!(more_open(&ran));
        let (paged, _) = resolve_find_intent(&opened, FindIntent::NextButton, false, true);
        assert!(more_open(&paged));
        let (cleared, _) = resolve_find_intent(&opened, FindIntent::ClearButton, false, false);
        assert!(!more_open(&cleared));
    }
}
