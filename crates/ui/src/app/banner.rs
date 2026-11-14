//! Localized banner text. Pure so it can be tested without egui.
use super::persistence::Recovery;
use super::session::session_display_name;
use super::state::{AppState, Banner};
use crate::I18n;
use crate::theme::{Scheme, icons};
use crate::viewmodel::paste_block_key;
use eframe::egui::Color32;

/// `(headline, detail)` for a banner; every string comes from the locale.
pub fn banner_text(banner: &Banner, i18n: &I18n) -> (String, Option<String>) {
    match banner {
        Banner::Failure { context, error } => (i18n.failure(context), Some(i18n.error(error))),
        Banner::Persist(e) => (i18n.t(e.i18n_key()), None),
        Banner::ToolsPersist(e) => (i18n.t(e.tools_i18n_key()), None),
        Banner::PipelinesPersist(e) => (i18n.t(e.i18n_key()), None),
        Banner::ImportFailed(e) => (i18n.t(e.i18n_key()), None),
        Banner::ExportFailed(_) => (i18n.t("banner.export_failed"), None),
        Banner::Recovery(r) => (recovery_headline(r, i18n), None),
        Banner::NotConnected => (i18n.t("banner.not_connected"), None),
        Banner::PasteBlocked(block) => (
            i18n.t("copy.banner.paste_blocked"),
            Some(i18n.t(paste_block_key(*block))),
        ),
        Banner::CopyUnavailable => (i18n.t("copy.menu.copy_unavailable"), None),
        Banner::HostForgotten => (i18n.t("banner.host_forgotten"), None),
        Banner::ProjectionReadOnly => (i18n.t("banner.projection_read_only"), None),
        Banner::ReadOnly => (i18n.t("banner.read_only"), None),
        Banner::CompareApplyReadOnly => (i18n.t("compare.apply.read_only_target"), None),
        Banner::CompareApplyReadOnlySource => (i18n.t("compare.apply.read_only_source"), None),
        Banner::FieldsConflict => (i18n.t("banner.fields_conflict"), None),
        Banner::FieldsCovered => (i18n.t("banner.fields_covered"), None),
        Banner::WriteNotSentConnectionChanged => {
            (i18n.t("banner.write_not_sent_connection_changed"), None)
        }
        Banner::WriteNotSentBusy => (i18n.t("banner.write_not_sent_busy"), None),
    }
}

fn recovery_headline(recovery: &Recovery, i18n: &I18n) -> String {
    let key = recovery.i18n_key();
    match recovery {
        Recovery::Skipped { count, .. } | Recovery::Repaired { count, .. } => {
            i18n.t_with(key, &[("n", &count.to_string())])
        }
        Recovery::SkippedAndRepaired {
            skipped, repaired, ..
        } => i18n.t_with(
            key,
            &[
                ("skipped", &skipped.to_string()),
                ("repaired", &repaired.to_string()),
            ],
        ),
        Recovery::Corrupt { .. }
        | Recovery::Unreadable { .. }
        | Recovery::Sampling { .. }
        | Recovery::Explain { .. }
        | Recovery::Tools { .. }
        | Recovery::Pipelines { .. }
        | Recovery::PipelinesNewer => i18n.t(key),
    }
}

/// How a banner reads: something failed, something was refused or skipped, or just a note.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BannerLevel {
    Error,
    Warning,
    Info,
}

/// The level of `banner`: failures are errors, a refused or partial action a warning, and
/// confirmations a note.
pub fn banner_level(banner: &Banner) -> BannerLevel {
    match banner {
        Banner::Failure { .. }
        | Banner::Persist(_)
        | Banner::ToolsPersist(_)
        | Banner::PipelinesPersist(_)
        | Banner::ImportFailed(_)
        | Banner::ExportFailed(_) => BannerLevel::Error,
        Banner::HostForgotten | Banner::FieldsCovered => BannerLevel::Info,
        Banner::Recovery(_)
        | Banner::NotConnected
        | Banner::PasteBlocked(_)
        | Banner::CopyUnavailable
        | Banner::ProjectionReadOnly
        | Banner::ReadOnly
        | Banner::CompareApplyReadOnly
        | Banner::CompareApplyReadOnlySource
        | Banner::FieldsConflict
        | Banner::WriteNotSentConnectionChanged
        | Banner::WriteNotSentBusy => BannerLevel::Warning,
    }
}

/// The icon of a banner level.
pub fn banner_icon(level: BannerLevel) -> char {
    match level {
        BannerLevel::Error => icons::ERROR,
        BannerLevel::Warning => icons::WARNING,
        BannerLevel::Info => icons::INFO,
    }
}

/// The i18n key naming a banner level, the accessible name of its icon.
pub fn banner_level_key(level: BannerLevel) -> &'static str {
    match level {
        BannerLevel::Error => "banner.level.error",
        BannerLevel::Warning => "banner.level.warning",
        BannerLevel::Info => "banner.level.info",
    }
}

/// (container, content) of a banner level: `error_container`, `warning_container` or
/// `secondary_container` with the matching `on_` role.
pub fn banner_colors(scheme: &Scheme, level: BannerLevel) -> (Color32, Color32) {
    match level {
        BannerLevel::Error => (scheme.error_container, scheme.on_error_container),
        BannerLevel::Warning => (scheme.warning_container, scheme.on_warning_container),
        BannerLevel::Info => (scheme.secondary_container, scheme.on_secondary_container),
    }
}

/// "In {name}:" when the banner was raised from a session event and that session's saved
/// connection still exists; none for banners nobody owns.
pub fn banner_prefix(state: &AppState, i18n: &I18n) -> Option<String> {
    let name = session_display_name(state, state.banner_owner()?, i18n)?;
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
    use crate::app::pipelines_store::PipelinesSaveError;
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
            Banner::ToolsPersist(PersistError::ConfigDirUnknown),
            Banner::ToolsPersist(PersistError::Write(std::io::ErrorKind::Other)),
            Banner::PipelinesPersist(PipelinesSaveError::Persist(PersistError::ConfigDirUnknown)),
            Banner::PipelinesPersist(PipelinesSaveError::Persist(PersistError::Write(
                std::io::ErrorKind::Other,
            ))),
            Banner::PipelinesPersist(PipelinesSaveError::TooLarge),
            Banner::PipelinesPersist(PipelinesSaveError::ChangedOnDisk),
            Banner::PipelinesPersist(PipelinesSaveError::UnreadableOnDisk),
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
            Banner::Recovery(Recovery::SkippedAndRepaired {
                skipped: 2,
                repaired: 3,
                backed_up: true,
            }),
            Banner::Recovery(Recovery::SkippedAndRepaired {
                skipped: 2,
                repaired: 3,
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
            Banner::Recovery(Recovery::Sampling { backed_up: true }),
            Banner::Recovery(Recovery::Sampling { backed_up: false }),
            Banner::Recovery(Recovery::Explain { backed_up: true }),
            Banner::Recovery(Recovery::Explain { backed_up: false }),
            Banner::Recovery(Recovery::Tools { backed_up: true }),
            Banner::Recovery(Recovery::Tools { backed_up: false }),
            Banner::Recovery(Recovery::Pipelines { backed_up: true }),
            Banner::Recovery(Recovery::Pipelines { backed_up: false }),
            Banner::Recovery(Recovery::PipelinesNewer),
            Banner::NotConnected,
            Banner::CopyUnavailable,
            Banner::HostForgotten,
            Banner::ProjectionReadOnly,
            Banner::ReadOnly,
            Banner::PasteBlocked(crate::viewmodel::PasteBlock::ReadOnlyTarget),
            Banner::CompareApplyReadOnly,
            Banner::CompareApplyReadOnlySource,
            Banner::FieldsConflict,
            Banner::FieldsCovered,
            Banner::WriteNotSentConnectionChanged,
            Banner::WriteNotSentBusy,
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

    #[test]
    fn a_file_with_skipped_and_adjusted_entries_says_both() {
        for backed_up in [true, false] {
            let (head, _) = resolved(&Banner::Recovery(Recovery::SkippedAndRepaired {
                skipped: 7,
                repaired: 4,
                backed_up,
            }));
            assert!(head.contains('7') && head.contains("not loaded"), "{head}");
            assert!(head.contains('4') && head.contains("adjusted"), "{head}");
            assert!(!head.contains('{'), "{head}");
        }
    }

    #[test]
    fn levels_pick_the_container_roles() {
        use crate::theme::{Mode, scheme};
        let light = scheme(Mode::Light);
        let failure = Banner::Failure {
            context: "find",
            error: CoreError::AuthFailed,
        };
        assert_eq!(banner_level(&failure), BannerLevel::Error);
        assert_eq!(banner_level(&Banner::ReadOnly), BannerLevel::Warning);
        assert_eq!(banner_level(&Banner::HostForgotten), BannerLevel::Info);
        assert_eq!(
            banner_colors(light, BannerLevel::Error),
            (light.error_container, light.on_error_container)
        );
        assert_eq!(
            banner_colors(light, BannerLevel::Warning),
            (light.warning_container, light.on_warning_container)
        );
        assert_eq!(
            banner_colors(light, BannerLevel::Info),
            (light.secondary_container, light.on_secondary_container)
        );
    }

    /// The banner's text and its text buttons (drawn in `primary`) are readable on every level.
    #[test]
    fn banner_text_and_buttons_meet_aa_in_both_modes() {
        use crate::theme::contrast::ratio;
        use crate::theme::{Mode, scheme};
        for mode in [Mode::Light, Mode::Dark] {
            let s = scheme(mode);
            for level in [BannerLevel::Error, BannerLevel::Warning, BannerLevel::Info] {
                let (container, content) = banner_colors(s, level);
                assert!(ratio(content, container) >= 4.5, "{mode:?} {level:?} text");
                assert!(
                    ratio(s.primary, container) >= 4.5,
                    "{mode:?} {level:?} button"
                );
            }
        }
    }
}
