//! The query bar's index verdict as a badge: tone, icon, title and reasons, all as i18n keys.
use crate::app::VerdictState;
use curlybrace_core::error::CoreError;
use curlybrace_core::explain::{Reason, Verdict, VerdictKind};
use curlybrace_core::store::Namespace;

/// How the badge reads at a glance. Never the only signal: the icon and title say it too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Good,
    Warn,
    Neutral,
    Unavailable,
    Checking,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BadgeModel {
    pub tone: Tone,
    pub icon: &'static str,
    pub title_key: &'static str,
    pub reasons: Vec<Reason>,
    /// A ready-to-copy `createIndex` command; only for a verdict that is not good.
    pub suggestion: Option<String>,
    /// The verdict is an estimate: the server returned a plan without execution counts.
    pub plan_only: bool,
}

const ICON_GOOD: &str = "\u{2714}";
const ICON_WARN: &str = "\u{26a0}";
const ICON_INFO: &str = "\u{2139}";
const ICON_WAIT: &str = "\u{2026}";

/// The badge for `state`, or `None` when there is nothing to show.
pub fn badge(state: &VerdictState, ns: Option<&Namespace>) -> Option<BadgeModel> {
    match state {
        VerdictState::None => None,
        VerdictState::Pending { .. } => Some(simple(
            Tone::Checking,
            ICON_WAIT,
            "query.verdict.checking",
            Vec::new(),
        )),
        VerdictState::Ready { verdict, .. } => Some(ready(verdict, ns)),
        VerdictState::Unavailable { error, .. } => Some(simple(
            Tone::Unavailable,
            ICON_INFO,
            "query.verdict.unavailable",
            vec![Reason::new(unavailable_reason(error))],
        )),
    }
}

fn simple(
    tone: Tone,
    icon: &'static str,
    title_key: &'static str,
    reasons: Vec<Reason>,
) -> BadgeModel {
    BadgeModel {
        tone,
        icon,
        title_key,
        reasons,
        suggestion: None,
        plan_only: false,
    }
}

/// The server detail is never shown: a view or unsupported target and any other failure get a
/// fixed, localized reason.
fn unavailable_reason(error: &CoreError) -> &'static str {
    match error {
        CoreError::Unsupported(_) => "unsupported",
        _ => "unavailable",
    }
}

fn ready(verdict: &Verdict, ns: Option<&Namespace>) -> BadgeModel {
    let (tone, icon, title_key) = match verdict.kind {
        VerdictKind::IndexUsed => (Tone::Good, ICON_GOOD, "query.verdict.title_index_used"),
        VerdictKind::CoveredByIndex => (Tone::Good, ICON_GOOD, "query.verdict.title_covered"),
        VerdictKind::CollectionScan => (Tone::Warn, ICON_WARN, "query.verdict.title_collscan"),
        VerdictKind::SortNotCovered => (Tone::Warn, ICON_WARN, "query.verdict.title_sort"),
        VerdictKind::InefficientIndex => (Tone::Warn, ICON_WARN, "query.verdict.title_inefficient"),
        VerdictKind::NoFilter => (Tone::Neutral, ICON_INFO, "query.verdict.title_no_filter"),
        VerdictKind::EmptyCollection => (Tone::Neutral, ICON_INFO, "query.verdict.title_empty"),
    };
    let suggestion = match (tone, ns) {
        (Tone::Good, _) | (_, None) => None,
        (_, Some(ns)) if !planned_on(verdict, ns) => None,
        (_, Some(ns)) => verdict.suggestion.as_ref().map(|s| s.command(ns)),
    };
    BadgeModel {
        tone,
        icon,
        title_key,
        reasons: verdict.reasons.clone(),
        suggestion,
        plan_only: verdict.plan_only,
    }
}

/// Whether the plan ran on `ns` itself. A view runs on its source collection, so an index
/// proposed for the view's name would be a command that fails; an unknown namespace is trusted.
fn planned_on(verdict: &Verdict, ns: &Namespace) -> bool {
    verdict
        .namespace
        .as_deref()
        .is_none_or(|planned| planned == format!("{}.{}", ns.db, ns.collection))
}

/// The full `en.json` key of a reason core (or `badge`) names by suffix. Spelled out as literals
/// so every key stays discoverable by the key-usage test.
pub fn reason_text_key(suffix: &str) -> Option<&'static str> {
    Some(match suffix {
        "collscan" => "query.verdict.reason_collscan",
        "sort_blocking" => "query.verdict.reason_sort_blocking",
        "inefficient" => "query.verdict.reason_inefficient",
        "no_filter" => "query.verdict.reason_no_filter",
        "empty_collection" => "query.verdict.reason_empty_collection",
        "unsupported" => "query.verdict.reason_unsupported",
        "unavailable" => "query.verdict.reason_unavailable",
        _ => return None,
    })
}

#[cfg(test)]
#[path = "verdict_tests.rs"]
mod tests;
