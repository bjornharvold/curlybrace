//! Which part of the app banner the edit dialog repeats: only a banner that is new since the
//! dialog opened. One that was already showing (a past connect failure, a persistence notice) is
//! about something else and stays in its normal place.
//!
//! The dialog's opening is detected from the `editing` flag by the screen that draws it, once
//! per frame, so every way in (New, Edit, Duplicate) and every way out (Cancel, Escape, Save,
//! delete, connect) is covered without touching them.
//!
//! The banner is compared by value, so an action that starts FROM the dialog and raises a banner
//! while the dialog stays open (a refused or failed Save) CLAIMS it with `claimed`: the baseline
//! is dropped, so the banner shows even when an identical one was showing at opening.
//!
//! Known limit: a banner raised by something other than the dialog (a background event) that is
//! IDENTICAL to the one showing at opening is still not considered new.
use super::state::Banner;

/// The banner showing when the dialog was last seen opening; default while it is closed.
#[derive(Debug, Default, PartialEq)]
pub struct DialogBannerWatch {
    open: bool,
    baseline: Option<Banner>,
}

impl DialogBannerWatch {
    /// The watch after a frame in which the dialog was `editing` (or not) with `banner` showing.
    /// Closing forgets the baseline; the first frame of an opening takes it.
    pub fn observed(self, editing: bool, banner: Option<&Banner>) -> Self {
        match (editing, self.open) {
            (false, _) => Self::default(),
            (true, true) => self,
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
        let still = open.observed(true, Some(&failure("b")));
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
}
