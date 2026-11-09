//! Fixtures for the browser with the index verdict chip showing.
use super::shots::browsing;
use curlybrace_core::explain::{Reason, SuggestedIndex, Verdict, VerdictKind};
use curlybrace_ui::app::VerdictState;
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::viewmodel::ViewMode;

/// The tree view of `crm.customers` whose last run was served by a collection scan: the "bad"
/// chip, with the "why" details closed.
pub fn collection_scan(state: &mut AppState) {
    browsing(ViewMode::Tree)(state);
    state.verdict = VerdictState::Ready {
        query: state.page_query.clone(),
        verdict: scan_verdict(),
    };
}

fn scan_verdict() -> Verdict {
    Verdict {
        kind: VerdictKind::CollectionScan,
        index_name: None,
        keys_examined: 0,
        docs_examined: 12_480,
        returned: 3,
        plan_only: false,
        reasons: vec![Reason::new("collscan")],
        suggestion: Some(SuggestedIndex {
            keys: vec![("status".into(), 1)],
        }),
        namespace: Some("crm.customers".into()),
    }
}
