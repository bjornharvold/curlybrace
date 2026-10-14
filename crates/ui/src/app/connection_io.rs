//! Import and export of the saved-connection list. Exports never carry secrets (the type has
//! no secret fields); imports never trust ids, names or sizes from the file.
use super::connections::{ConfigPath, persist, refused};
use super::persistence::{PersistError, atomic_write, parse_connections};
use super::state::{AppState, Banner};
use crate::views::connections::{ConnectionsState, Notice};
use curlybrace_core::connection::{ConnectionConfig, unique_name};
use std::path::Path;
use uuid::Uuid;

pub const MAX_IMPORT_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_IMPORT_ENTRIES: usize = 2000;

#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    pub connections: Vec<ConnectionConfig>,
    pub skipped: usize,
}

pub fn export_bytes(list: &[ConnectionConfig]) -> Result<Vec<u8>, PersistError> {
    serde_json::to_vec_pretty(list)
        .map_err(|_| PersistError::Write(std::io::ErrorKind::InvalidData))
}

pub fn prepare_import(
    bytes: &[u8],
    existing: &[ConnectionConfig],
    now: i64,
) -> Result<Imported, PersistError> {
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err(PersistError::Corrupt);
    }
    let entries = serde_json::from_slice::<Vec<serde_json::Value>>(bytes)
        .map_err(|_| PersistError::Corrupt)?
        .len();
    if entries > MAX_IMPORT_ENTRIES {
        return Err(PersistError::Corrupt);
    }
    let loaded = parse_connections(bytes)?;
    let mut taken = existing.to_vec();
    let mut fresh = Vec::with_capacity(loaded.connections.len());
    for c in loaded.connections {
        let name = unique_name(&c.name, &taken);
        let c = ConnectionConfig {
            id: Uuid::new_v4(),
            name,
            created_at: Some(now),
            updated_at: Some(now),
            last_connected: None,
            ..c
        };
        taken.push(c.clone());
        fresh.push(c);
    }
    Ok(Imported {
        connections: fresh,
        skipped: loaded.skipped,
    })
}

pub(super) fn import_into(state: AppState, file: &Path, now: i64, config: &ConfigPath) -> AppState {
    if let Err(e) = state.persist.check() {
        return refused(state, e);
    }
    let read = std::fs::metadata(file)
        .map_err(|e| PersistError::Read(e.kind()))
        .and_then(|m| {
            if m.len() > MAX_IMPORT_BYTES as u64 {
                Err(PersistError::Corrupt)
            } else {
                std::fs::read(file).map_err(|e| PersistError::Read(e.kind()))
            }
        });
    let imported = read.and_then(|bytes| prepare_import(&bytes, &state.conn.connections, now));
    match imported {
        Ok(imported) => {
            let count = imported.connections.len();
            let mut list = state.conn.connections.clone();
            list.extend(imported.connections);
            let banner = persist(config, &list).err().map(Banner::Persist);
            AppState {
                conn: ConnectionsState {
                    connections: list,
                    // No success notice next to a save error: the banner tells the story.
                    notice: banner.is_none().then_some(Notice::Imported {
                        imported: count,
                        skipped: imported.skipped,
                    }),
                    ..state.conn
                },
                banner: banner.or(state.banner),
                ..state
            }
        }
        Err(e) => AppState {
            banner: Some(Banner::Persist(e)),
            ..state
        },
    }
}

pub(super) fn export_to(state: AppState, file: &Path) -> AppState {
    let result = export_bytes(&state.conn.connections).and_then(|b| atomic_write(file, &b));
    match result {
        Ok(()) => {
            let count = state.conn.connections.len();
            AppState {
                conn: ConnectionsState {
                    notice: Some(Notice::Exported { count }),
                    ..state.conn
                },
                ..state
            }
        }
        Err(e) => AppState {
            banner: Some(Banner::Persist(e)),
            ..state
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::persistence::PersistGuard;

    fn cfg(name: &str) -> ConnectionConfig {
        ConnectionConfig::new(name, "mongodb://h")
    }

    #[test]
    fn export_has_no_password_field_and_round_trips_through_import() {
        let mut a = cfg("a");
        a.username = Some("bob".into());
        let bytes = export_bytes(&[a.clone()]).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(!text.to_lowercase().contains("password"));
        let imported = prepare_import(&bytes, &[], 9).unwrap();
        assert_eq!(imported.connections.len(), 1);
        assert_eq!(imported.connections[0].username.as_deref(), Some("bob"));
    }

    #[test]
    fn import_assigns_fresh_ids_even_when_the_file_repeats_an_existing_id() {
        let existing = cfg("a");
        let bytes = export_bytes(&[existing.clone(), existing.clone()]).unwrap();
        let imported = prepare_import(&bytes, &[existing.clone()], 9).unwrap();
        let ids: std::collections::HashSet<_> = imported.connections.iter().map(|c| c.id).collect();
        assert_eq!(ids.len(), 2);
        assert!(!ids.contains(&existing.id));
    }

    #[test]
    fn import_makes_names_unique_and_resets_connect_history() {
        let mut a = cfg("a");
        a.last_connected = Some(5);
        let bytes = export_bytes(&[a.clone(), a.clone()]).unwrap();
        let imported = prepare_import(&bytes, &[a], 9).unwrap();
        let names: Vec<_> = imported
            .connections
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(names, vec!["a 2", "a 3"]);
        assert!(
            imported
                .connections
                .iter()
                .all(|c| c.last_connected.is_none())
        );
        assert!(imported.connections.iter().all(|c| c.created_at == Some(9)));
    }

    #[test]
    fn import_skips_invalid_entries_and_counts_them() {
        let bytes = br#"[{"nope":1}, 7]"#;
        let imported = prepare_import(bytes, &[], 9).unwrap();
        assert_eq!((imported.connections.len(), imported.skipped), (0, 2));
    }

    #[test]
    fn import_rejects_non_json_and_oversized_input() {
        assert_eq!(
            prepare_import(b"not json", &[], 1).unwrap_err(),
            PersistError::Corrupt
        );
        let big = vec![b' '; MAX_IMPORT_BYTES + 1];
        assert!(prepare_import(&big, &[], 1).is_err());
    }

    #[test]
    fn import_rejects_too_many_entries() {
        let many = format!("[{}]", vec!["0"; MAX_IMPORT_ENTRIES + 1].join(","));
        assert!(prepare_import(many.as_bytes(), &[], 1).is_err());
    }

    #[test]
    fn hostile_folder_entries_are_skipped_by_validation() {
        let mut bad = cfg("a");
        bad.folder = Some("x\ny".into());
        let bytes = serde_json::to_vec(&[bad]).unwrap();
        assert_eq!(prepare_import(&bytes, &[], 1).unwrap().skipped, 1);
    }

    #[test]
    fn hostile_name_entries_are_skipped_by_validation() {
        let huge = cfg(&"n".repeat(1024 * 1024));
        let bytes =
            serde_json::to_vec(&[cfg("a\nb"), cfg("tab\there"), huge, cfg("good")]).unwrap();
        assert!(bytes.len() < MAX_IMPORT_BYTES);
        let imported = prepare_import(&bytes, &[], 1).unwrap();
        assert_eq!(imported.skipped, 3);
        assert_eq!(imported.connections.len(), 1);
        assert_eq!(imported.connections[0].name, "good");
    }

    #[test]
    fn a_ten_megabyte_name_is_stopped_by_the_file_size_cap() {
        let bytes = serde_json::to_vec(&[cfg(&"n".repeat(10 * 1024 * 1024))]).unwrap();
        assert_eq!(
            prepare_import(&bytes, &[], 1).unwrap_err(),
            PersistError::Corrupt
        );
    }

    #[test]
    fn a_failed_save_keeps_the_entries_but_shows_no_success_notice() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("in.json");
        std::fs::write(&file, export_bytes(&[cfg("x")]).unwrap()).unwrap();
        // A path whose parent is a file cannot be written.
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"x").unwrap();
        let config: ConfigPath = Ok(blocker.join("connections.json"));
        let state = import_into(AppState::new(vec![cfg("a")]), &file, 7, &config);
        assert_eq!(state.conn.connections.len(), 2);
        assert!(matches!(state.banner, Some(Banner::Persist(_))));
        assert!(state.conn.notice.is_none());
    }

    fn setup() -> (tempfile::TempDir, ConfigPath) {
        let dir = tempfile::tempdir().unwrap();
        let path: ConfigPath = Ok(dir.path().join("connections.json"));
        (dir, path)
    }

    #[test]
    fn import_into_adds_persists_and_sets_the_notice() {
        let (dir, config) = setup();
        let file = dir.path().join("in.json");
        std::fs::write(&file, export_bytes(&[cfg("x"), cfg("y")]).unwrap()).unwrap();
        let state = import_into(AppState::new(vec![cfg("a")]), &file, 7, &config);
        assert_eq!(state.conn.connections.len(), 3);
        assert_eq!(
            state.conn.notice,
            Some(Notice::Imported {
                imported: 2,
                skipped: 0
            })
        );
        assert!(state.banner.is_none());
        let saved = std::fs::read_to_string(config.as_ref().unwrap()).unwrap();
        assert!(saved.contains("\"x\"") && saved.contains("\"y\""));
    }

    #[test]
    fn import_into_with_a_blocked_guard_changes_nothing_and_shows_a_banner() {
        let (dir, config) = setup();
        let file = dir.path().join("in.json");
        std::fs::write(&file, export_bytes(&[cfg("x")]).unwrap()).unwrap();
        let mut start = AppState::new(vec![cfg("a")]);
        start.persist = PersistGuard::blocked();
        let state = import_into(start, &file, 7, &config);
        assert_eq!(state.conn.connections.len(), 1);
        assert!(state.conn.notice.is_none());
        assert_eq!(state.banner, Some(Banner::Persist(PersistError::Blocked)));
        assert!(!config.as_ref().unwrap().exists());
    }

    #[test]
    fn import_into_rejects_an_oversized_file_without_reading_it() {
        let (dir, config) = setup();
        let file = dir.path().join("big.json");
        std::fs::write(&file, vec![b' '; MAX_IMPORT_BYTES + 1]).unwrap();
        let state = import_into(AppState::new(vec![cfg("a")]), &file, 7, &config);
        assert_eq!(state.conn.connections.len(), 1);
        assert!(state.banner.is_some());
    }

    #[cfg(unix)]
    #[test]
    fn export_to_writes_a_private_file_with_the_names() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("out.json");
        let state = export_to(AppState::new(vec![cfg("alpha"), cfg("beta")]), &file);
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("alpha") && text.contains("beta"));
        let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(state.conn.notice, Some(Notice::Exported { count: 2 }));
    }
}
