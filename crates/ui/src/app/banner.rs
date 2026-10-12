//! Localized banner text. Pure so it can be tested without egui.
use super::persistence::Recovery;
use super::state::Banner;
use crate::I18n;

/// `(headline, detail)` for a banner; every string comes from the locale.
pub fn banner_text(banner: &Banner, i18n: &I18n) -> (String, Option<String>) {
    match banner {
        Banner::Failure { context, error } => (i18n.failure(context), Some(i18n.error(error))),
        Banner::Persist(e) => (i18n.t(e.i18n_key()), None),
        Banner::Recovery(r) => {
            let count = match r {
                Recovery::Skipped { count, .. } => *count,
                _ => 0,
            };
            (
                i18n.t_with(r.i18n_key(), &[("n", &count.to_string())]),
                None,
            )
        }
        Banner::NotConnected => (i18n.t("banner.not_connected"), None),
        Banner::HostForgotten => (i18n.t("banner.host_forgotten"), None),
        Banner::ProjectionReadOnly => (i18n.t("banner.projection_read_only"), None),
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
            Banner::Persist(PersistError::InvalidPath),
            Banner::Persist(PersistError::Blocked),
            Banner::Recovery(Recovery::Corrupt { backed_up: true }),
            Banner::Recovery(Recovery::Corrupt { backed_up: false }),
            Banner::Recovery(Recovery::Unreadable { backed_up: true }),
            Banner::Recovery(Recovery::Unreadable { backed_up: false }),
            Banner::Recovery(Recovery::Skipped {
                count: 2,
                backed_up: true,
            }),
            Banner::Recovery(Recovery::Skipped {
                count: 2,
                backed_up: false,
            }),
            Banner::NotConnected,
            Banner::HostForgotten,
            Banner::ProjectionReadOnly,
        ];
        for b in &banners {
            let (head, _) = resolved(b);
            assert!(!head.contains('⟦'), "{b:?}");
        }
        for context in ["save_connection", "secrets", "runtime", "delete_secrets"] {
            let (head, _) = resolved(&Banner::Failure {
                context,
                error: CoreError::NotFound,
            });
            assert!(!head.contains('⟦'), "{context}");
        }
    }

    #[test]
    fn skipped_count_is_interpolated() {
        let (head, _) = resolved(&Banner::Recovery(Recovery::Skipped {
            count: 3,
            backed_up: true,
        }));
        assert!(head.contains('3'));
    }
}
