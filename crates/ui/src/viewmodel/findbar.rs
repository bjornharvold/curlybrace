//! Find bar state: raw query text plus the last validation error.
use mongostudio_core::error::CoreError;
use mongostudio_core::query::{QueryText, parse_query};
use mongostudio_core::store::Query;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FindBarState {
    pub text: QueryText,
    pub last_error: Option<CoreError>,
}

impl FindBarState {
    /// Same text, no error (used by the view after a successful submit).
    pub fn cleared_error(&self) -> Self {
        Self {
            text: self.text.clone(),
            last_error: None,
        }
    }
}

/// Parses the bar text. On failure returns a new state carrying the error with the text
/// untouched, so the user can fix what they typed.
#[allow(clippy::result_large_err)] // signature mandated: Err carries the full new state
pub fn submit_find(state: &FindBarState) -> Result<Query, FindBarState> {
    parse_query(&state.text).map_err(|e| FindBarState {
        text: state.text.clone(),
        last_error: Some(e),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongostudio_core::store::PAGE_SIZE;

    fn with_filter(f: &str) -> FindBarState {
        FindBarState {
            text: QueryText {
                filter: f.into(),
                ..Default::default()
            },
            last_error: Some(CoreError::InvalidJson("old".into())),
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
}
