//! Which part of the app banner the edit dialog repeats: only a banner that is new since the
//! dialog opened. One that was already showing (a past connect failure, a persistence notice) is
//! about something else and stays in its normal place.
//!
//! The dialog's opening is detected from the `editing` flag by the screen that draws it, once
//! per frame, so every way in (New, Edit, Duplicate) and every way out (Cancel, Escape, Save,
//! delete, connect) is covered without touching them.
//!
//! Known limit: the banner is compared by value, so an IDENTICAL banner raised again after the
//! dialog opened (the same failure twice) is not considered new. A counter on every banner
//! assignment would fix that at the cost of touching every place that raises one.
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
    fn opening_with_no_banner_makes_any_later_banner_new() {
        let watch = DialogBannerWatch::default().observed(true, None);
        assert!(watch.is_new(&failure("a")));
    }
}
