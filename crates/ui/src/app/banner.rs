//! Localized banner text. Pure so it can be tested without egui.
use super::state::Banner;
use crate::I18n;

/// `(headline, detail)` for a banner; every string comes from the locale.
pub fn banner_text(banner: &Banner, i18n: &I18n) -> (String, Option<String>) {
    match banner {
        Banner::Failure { context, error } => (i18n.failure(context), Some(i18n.error(error))),
        Banner::Persist(e) => (i18n.t(e.i18n_key()), None),
        Banner::SkippedEntries(n) => (
            i18n.t_with("persist.skipped_entries", &[("n", &n.to_string())]),
            None,
        ),
        Banner::NotConnected => (i18n.t("banner.not_connected"), None),
        Banner::HostForgotten => (i18n.t("banner.host_forgotten"), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::persistence::PersistError;
    use mongostudio_core::error::CoreError;

    fn resolved(b: &Banner) -> (String, Option<String>) {
        banner_text(b, &I18n::english())
    }

    #[test]
    fn failure_combines_headline_and_error_copy() {
        let (head, detail) = resolved(&Banner::Failure {
            context: "find",
            error: CoreError::Database("x".into()),
        });
        assert_eq!(head, "Could not load documents");
        assert!(detail.unwrap().contains("database"));
    }

    #[test]
    fn every_banner_and_failure_context_has_copy() {
        let banners = [
            Banner::Persist(PersistError::ConfigDirUnknown),
            Banner::Persist(PersistError::Read(std::io::ErrorKind::Other)),
            Banner::Persist(PersistError::Corrupt),
            Banner::Persist(PersistError::Write(std::io::ErrorKind::Other)),
            Banner::SkippedEntries(2),
            Banner::NotConnected,
            Banner::HostForgotten,
        ];
        for b in &banners {
            let (head, _) = resolved(b);
            assert!(!head.contains('⟦'), "{b:?}");
        }
        for context in ["save_connection", "secrets", "runtime"] {
            let (head, _) = resolved(&Banner::Failure {
                context,
                error: CoreError::NotFound,
            });
            assert!(!head.contains('⟦'), "{context}");
        }
    }

    #[test]
    fn skipped_count_is_interpolated() {
        let (head, _) = resolved(&Banner::SkippedEntries(3));
        assert!(head.contains('3'));
    }
}
