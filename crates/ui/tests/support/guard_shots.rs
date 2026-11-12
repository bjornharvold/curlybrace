//! Fixtures for the browser with the index-miss guard's notice under the find bar.
use super::shots::browsing;
use curlybrace_core::explain::{Reason, SuggestedIndex, VerdictKind};
use curlybrace_core::guard::{GuardFinding, GuardReport};
use curlybrace_core::store::Namespace;
use curlybrace_ui::app::GuardState;
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::viewmodel::ViewMode;

/// The tree view of `crm.customers` with a Run held back because its filter would scan the whole
/// collection: the warning with the reason, the suggested index, Run anyway and Cancel.
pub fn scan_notice(state: &mut AppState) {
    browsing(ViewMode::Tree)(state);
    let query = state.page_query.clone();
    state.guard = Box::new(GuardState::Warn {
        query: query.clone(),
        report: GuardReport {
            findings: vec![GuardFinding {
                ns: Namespace::new("crm", "customers"),
                query,
                kind: VerdictKind::CollectionScan,
                reasons: vec![Reason::new("collscan")],
                suggestion: Some(SuggestedIndex {
                    keys: vec![("status".into(), 1)],
                }),
            }],
            unchecked: 0,
        },
    });
}
