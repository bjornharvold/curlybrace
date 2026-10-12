//! What the preflight check tells the user, and whether a run may start.
use curlybrace_core::compare::protocol::{Preflight, Side};
use curlybrace_core::store::IndexStatus;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PreflightView {
    /// Never checked, or the form changed since the last check.
    #[default]
    Unchecked,
    Checking,
    Ok,
    /// This side has no usable index and is too large to compare without one.
    NeedsIndex {
        side: Side,
    },
    /// Both sides are empty (a run is allowed and finds nothing).
    EmptyBoth,
}

/// An i18n key plus the key of the translated side name to substitute as `{side}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreflightMessage {
    pub key: &'static str,
    pub side_key: Option<&'static str>,
}

pub fn side_key(side: Side) -> &'static str {
    match side {
        Side::Source => "compare.side.source",
        Side::Target => "compare.side.target",
    }
}

impl From<&Preflight> for PreflightView {
    fn from(p: &Preflight) -> Self {
        if p.unindexed_too_large {
            return Self::NeedsIndex {
                side: unindexed_side(p),
            };
        }
        if p.source_count == 0 && p.target_count == 0 {
            return Self::EmptyBoth;
        }
        Self::Ok
    }
}

/// The unindexed side to blame: the larger one when both lack an index (source on a tie).
fn unindexed_side(p: &Preflight) -> Side {
    let source = p.source_index == IndexStatus::NotIndexed;
    let target = p.target_index == IndexStatus::NotIndexed;
    match (source, target) {
        (true, true) if p.target_count > p.source_count => Side::Target,
        (false, true) => Side::Target,
        _ => Side::Source,
    }
}

impl PreflightView {
    pub fn allows_start(&self) -> bool {
        matches!(self, Self::Ok | Self::EmptyBoth)
    }

    pub fn message(&self) -> Option<PreflightMessage> {
        match self {
            Self::NeedsIndex { side } => Some(PreflightMessage {
                key: "compare.preflight.needs_index",
                side_key: Some(side_key(*side)),
            }),
            Self::EmptyBoth => Some(PreflightMessage {
                key: "compare.preflight.empty_both",
                side_key: None,
            }),
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "preflight_tests.rs"]
mod tests;
