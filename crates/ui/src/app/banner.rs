//! Localized banner text. Pure so it can be tested without egui.
use super::persistence::Recovery;
use super::session::session_display_name;
use super::state::{AppState, Banner};
use crate::I18n;

/// `(headline, detail)` for a banner; every string comes from the locale.
pub fn banner_text(banner: &Banner, i18n: &I18n) -> (String, Option<String>) {
    match banner {
        Banner::Failure { context, error } => (i18n.failure(context), Some(i18n.error(error))),
        Banner::Persist(e) => (i18n.t(e.i18n_key()), None),
        Banner::ImportFailed(e) => (i18n.t(e.i18n_key()), None),
        Banner::ExportFailed(_) => (i18n.t("banner.export_failed"), None),
        Banner::Recovery(r) => {
            let count = match r {
                Recovery::Skipped { count, .. } | Recovery::Repaired { count, .. } => *count,
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
        Banner::FieldsConflict => (i18n.t("banner.fields_conflict"), None),
        Banner::FieldsCovered => (i18n.t("banner.fields_covered"), None),
    }
}

/// "In {name}:" when the banner was raised from a session event and that session's saved
/// connection still exists; none for banners nobody owns.
pub fn banner_prefix(state: &AppState, i18n: &I18n) -> Option<String> {
    let name = session_display_name(state, state.banner_owner()?)?;
    Some(i18n.t_with("banner.in_connection", &[("name", &name)]))
}

/// Raw server/user text to show weakly under a failure banner (see `error_detail`).
pub fn banner_detail(banner: &Banner) -> Option<&str> {
    match banner {
        Banner::Failure { error, .. } => crate::error_text::error_detail(error),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::connection_io::ImportError;
    use crate::app::persistence::PersistError;
    use curlybrace_core::error::CoreError;

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
            Banner::ImportFailed(ImportError::TooLarge),
            Banner::ImportFailed(ImportError::NotValid),
            Banner::ImportFailed(ImportError::TooManyEntries),
            Banner::ImportFailed(ImportError::Unreadable),
            Banner::ExportFailed(PersistError::Write(std::io::ErrorKind::Other)),
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
            Banner::Recovery(Recovery::Repaired {
                count: 2,
                backed_up: true,
            }),
            Banner::Recovery(Recovery::Repaired {
                count: 2,
                backed_up: false,
            }),
            Banner::NotConnected,
            Banner::HostForgotten,
            Banner::ProjectionReadOnly,
            Banner::FieldsConflict,
            Banner::FieldsCovered,
        ];
        for b in &banners {
            let (head, _) = resolved(b);
            assert!(!head.contains('⟦'), "{b:?}");
        }
        for context in [
            "save_connection",
            "secrets",
            "runtime",
            "delete_secrets",
            "compare_open",
            "compare_databases",
            "compare_collections",
            "compare_check",
            "compare_start",
            "compare_rows",
            "compare_detail",
            "compare_duplicates",
            "compare_duplicates_timeout",
        ] {
            let (head, _) = resolved(&Banner::Failure {
                context,
                error: CoreError::NotFound,
            });
            assert!(!head.contains('⟦'), "{context}");
        }
    }

    #[test]
    fn only_database_style_failures_expose_a_banner_detail() {
        let db = Banner::Failure {
            context: "insert",
            error: CoreError::Database("E11000 duplicate key".into()),
        };
        assert_eq!(banner_detail(&db), Some("E11000 duplicate key"));
        let ssh = Banner::Failure {
            context: "connect",
            error: CoreError::Ssh("x".into()),
        };
        assert_eq!(banner_detail(&ssh), None);
        assert_eq!(banner_detail(&Banner::NotConnected), None);
    }

    #[test]
    fn import_and_export_banners_talk_about_the_file_not_the_saved_list() {
        for e in [
            ImportError::TooLarge,
            ImportError::NotValid,
            ImportError::TooManyEntries,
            ImportError::Unreadable,
        ] {
            let (head, _) = resolved(&Banner::ImportFailed(e));
            assert!(!head.contains("saved connections"), "{head}");
            assert!(head.contains("file"), "{head}");
        }
        let (head, _) = resolved(&Banner::ExportFailed(PersistError::Corrupt));
        assert!(!head.contains("saved connections"), "{head}");
    }

    #[test]
    fn skipped_count_is_interpolated() {
        let (head, _) = resolved(&Banner::Recovery(Recovery::Skipped {
            count: 3,
            backed_up: true,
        }));
        assert!(head.contains('3'));
    }

    #[test]
    fn repaired_banner_says_names_were_adjusted_not_lost() {
        let (head, _) = resolved(&Banner::Recovery(Recovery::Repaired {
            count: 3,
            backed_up: true,
        }));
        assert!(head.contains('3') && head.contains("adjusted"), "{head}");
        assert!(!head.contains("skipped"), "{head}");
    }
}
