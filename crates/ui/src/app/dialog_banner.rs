//! Which part of the app banner the edit dialog repeats: only a banner that is new since the
//! dialog opened. One that was already showing (a past connect failure, a persistence notice) is
//! about something else and stays in its normal place.
//!
//! The dialog's opening is detected from the `editing` flag by the screen that draws it, once
//! per frame, so every way in (New, Edit; Duplicate hands the watch over itself, see below) and
//! every way out (Cancel, Escape, Save, delete, connect) is covered without touching them.
//!
//! The banner is compared by value, so an action that starts FROM the dialog and raises a banner
//! while the dialog stays open (a refused or failed Save) CLAIMS it with `claimed`: the baseline
//! is dropped, so the banner shows even when an identical one was showing at opening.
//!
//! The same goes for an action that OPENS the dialog and raises a banner in that very step
//! (Duplicate when reading the stored secret flags fails): it hands the watch over already open
//! with no baseline, so that banner counts as new, while one showing before the action stays hidden.
//!
//! Known limit: a banner raised by something other than the dialog (a background event) that is
//! IDENTICAL to the one showing at opening, with no frame in between that showed anything
//! else, is still not considered new.
use super::state::Banner;

/// The banner that must not be repeated in the dialog: the one showing when it opened (for
/// Duplicate, the one from before the action); default while it is closed.
#[derive(Debug, Default, PartialEq)]
pub struct DialogBannerWatch {
    open: bool,
    baseline: Option<Banner>,
}

impl DialogBannerWatch {
    /// The watch after a frame in which the dialog was `editing` (or not) with `banner` showing.
    /// Closing forgets the baseline; the first frame of an opening takes it. While open, any
    /// frame whose banner differs from the baseline (none, or another one) drops it for good:
    /// the baseline banner is gone, so one that appears later is a new event even if equal.
    pub fn observed(self, editing: bool, banner: Option<&Banner>) -> Self {
        match (editing, self.open) {
            (false, _) => Self::default(),
            (true, true) if self.baseline.as_ref() == banner => self,
            (true, true) => Self {
                baseline: None,
                ..self
            },
            (true, false) => Self {
                open: true,
                baseline: banner.cloned(),
            },
        }
    }

    /// The watch after the dialog's own action raised the banner now showing while the dialog
    /// stays open: nothing is "already showing" any more, so that banner counts as new. A
    /// closed dialog is unaffected.
    pub fn claimed(self) -> Self {
        Self {
            baseline: None,
            ..self
        }
    }

    /// Whether the dialog should repeat `banner`: it is open and the banner is not the one that
    /// was already showing when it opened.
    pub fn is_new(&self, banner: &Banner) -> bool {
        self.open && self.baseline.as_ref() != Some(banner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use curlybrace_core::error::CoreError;

    fn failure(context: &'static str) -> Banner {
        Banner::Failure {
            context,
            error: CoreError::AuthFailed,
        }
    }

    #[test]
    fn a_closed_dialog_repeats_nothing() {
        let watch = DialogBannerWatch::default().observed(false, Some(&failure("a")));
        assert!(!watch.is_new(&failure("a")));
        assert!(!watch.is_new(&failure("b")));
    }

    #[test]
    fn the_banner_at_opening_is_not_new_but_a_different_one_is() {
        let watch = DialogBannerWatch::default().observed(true, Some(&failure("a")));
        assert!(!watch.is_new(&failure("a")));
        assert!(watch.is_new(&failure("b")));
    }

    #[test]
    fn the_baseline_is_kept_while_open_and_dropped_on_close() {
        let open = DialogBannerWatch::default().observed(true, Some(&failure("a")));
        let still = open.observed(true, Some(&failure("a")));
        assert!(
            !still.is_new(&failure("a")),
            "baseline survives later frames"
        );
        let closed = still.observed(false, Some(&failure("b")));
        assert_eq!(closed, DialogBannerWatch::default());
        let reopened = closed.observed(true, Some(&failure("b")));
        assert!(
            !reopened.is_new(&failure("b")),
            "the new opening has a new baseline"
        );
    }

    #[test]
    fn a_claimed_banner_is_new_even_if_identical_to_the_baseline() {
        let open = DialogBannerWatch::default().observed(true, Some(&failure("a")));
        assert!(!open.is_new(&failure("a")));
        let claimed = open.claimed();
        assert!(claimed.is_new(&failure("a")));
        assert!(
            claimed
                .observed(true, Some(&failure("a")))
                .is_new(&failure("a"))
        );
    }

    #[test]
    fn claiming_while_closed_changes_nothing_and_closing_forgets_the_claim() {
        assert_eq!(
            DialogBannerWatch::default().claimed(),
            DialogBannerWatch::default()
        );
        let claimed = DialogBannerWatch::default()
            .observed(true, Some(&failure("a")))
            .claimed();
        let reopened = claimed
            .observed(false, None)
            .observed(true, Some(&failure("a")));
        assert!(!reopened.is_new(&failure("a")));
    }

    #[test]
    fn opening_with_no_banner_makes_any_later_banner_new() {
        let watch = DialogBannerWatch::default().observed(true, None);
        assert!(watch.is_new(&failure("a")));
    }

    #[test]
    fn a_banner_raised_again_after_the_baseline_went_away_is_new() {
        let open = DialogBannerWatch::default().observed(true, Some(&failure("b")));
        let cleared = open.observed(true, None);
        let again = cleared.observed(true, Some(&failure("b")));
        assert!(again.is_new(&failure("b")));
    }

    #[test]
    fn a_baseline_replaced_by_another_banner_is_dropped_for_good() {
        let open = DialogBannerWatch::default().observed(true, Some(&failure("a")));
        let shown = open.observed(true, Some(&failure("b")));
        assert!(shown.is_new(&failure("b")));
        let back = shown.observed(true, Some(&failure("a")));
        assert!(back.is_new(&failure("a")));
    }

    #[test]
    fn a_baseline_banner_that_keeps_showing_stays_hidden() {
        let mut watch = DialogBannerWatch::default().observed(true, Some(&failure("a")));
        for _ in 0..3 {
            watch = watch.observed(true, Some(&failure("a")));
            assert!(!watch.is_new(&failure("a")));
        }
    }
}
