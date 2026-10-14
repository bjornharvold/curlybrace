//! Pure helpers for the one-level folder grouping of saved connections. Every function returns
//! a new list; the input is never mutated.
use crate::connection::{ConnectionConfig, MAX_FOLDER_LEN, is_forbidden_label_char};
use crate::error::{CoreError, Result};
use std::collections::BTreeSet;
use uuid::Uuid;

/// Trimmed folder name, or `None` for blank input (the root). Rejects over-long names,
/// control and invisible format characters.
pub fn normalize_folder(raw: &str) -> Result<Option<String>> {
    let name = raw.trim();
    if name.is_empty() {
        return Ok(None);
    }
    if name.chars().count() > MAX_FOLDER_LEN || name.chars().any(is_forbidden_label_char) {
        return Err(CoreError::InvalidUri("invalid folder name".into()));
    }
    Ok(Some(name.to_owned()))
}

/// The distinct non-root folder names, sorted.
pub fn folder_names(list: &[ConnectionConfig]) -> Vec<String> {
    list.iter()
        .filter_map(|c| c.folder.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn restamped(c: &ConnectionConfig, folder: Option<String>, now: i64) -> ConnectionConfig {
    ConnectionConfig {
        folder,
        updated_at: Some(now),
        ..c.clone()
    }
}

pub fn moved_to_folder(
    list: &[ConnectionConfig],
    id: Uuid,
    folder: Option<String>,
    now: i64,
) -> Vec<ConnectionConfig> {
    list.iter()
        .map(|c| {
            if c.id == id {
                restamped(c, folder.clone(), now)
            } else {
                c.clone()
            }
        })
        .collect()
}

/// Renames `from` to `to`; if `to` already exists the two folders merge.
pub fn renamed_folder(
    list: &[ConnectionConfig],
    from: &str,
    to: &str,
    now: i64,
) -> Result<Vec<ConnectionConfig>> {
    let target =
        normalize_folder(to)?.ok_or_else(|| CoreError::InvalidUri("invalid folder name".into()))?;
    Ok(list
        .iter()
        .map(|c| match c.folder.as_deref() {
            Some(f) if f == from => restamped(c, Some(target.clone()), now),
            _ => c.clone(),
        })
        .collect())
}

/// Moves the folder's members to the root. Never deletes a connection.
pub fn deleted_folder(list: &[ConnectionConfig], name: &str, now: i64) -> Vec<ConnectionConfig> {
    list.iter()
        .map(|c| match c.folder.as_deref() {
            Some(f) if f == name => restamped(c, None, now),
            _ => c.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn in_folder(name: &str, folder: Option<&str>) -> ConnectionConfig {
        let mut c = ConnectionConfig::new(name, "mongodb://h");
        c.folder = folder.map(str::to_owned);
        c
    }

    #[test]
    fn normalize_trims_and_maps_blank_to_none() {
        assert_eq!(normalize_folder("  work ").unwrap(), Some("work".into()));
        assert_eq!(normalize_folder("   ").unwrap(), None);
        assert!(normalize_folder("a\tb").is_err());
        for ch in ['\u{202E}', '\u{200B}', '\u{2066}', '\u{FEFF}', '\u{061C}'] {
            assert!(normalize_folder(&format!("a{ch}b")).is_err());
        }
        assert_eq!(normalize_folder("日本語").unwrap(), Some("日本語".into()));
        assert!(normalize_folder(&"x".repeat(MAX_FOLDER_LEN + 1)).is_err());
    }

    #[test]
    fn folder_names_are_sorted_unique_and_skip_the_root() {
        let list = [
            in_folder("a", Some("zeta")),
            in_folder("b", Some("alpha")),
            in_folder("c", Some("alpha")),
            in_folder("d", None),
        ];
        assert_eq!(
            folder_names(&list),
            vec!["alpha".to_owned(), "zeta".to_owned()]
        );
    }

    #[test]
    fn move_changes_only_that_connection_and_stamps_it() {
        let a = in_folder("a", None);
        let b = in_folder("b", None);
        let out = moved_to_folder(&[a.clone(), b.clone()], a.id, Some("work".into()), 77);
        assert_eq!(out[0].folder.as_deref(), Some("work"));
        assert_eq!(out[0].updated_at, Some(77));
        assert_eq!(out[1], b, "the other connection is untouched");
        assert_eq!(a.folder, None, "the input is not mutated");
    }

    #[test]
    fn rename_moves_every_member_and_merges_into_an_existing_folder() {
        let list = [
            in_folder("a", Some("old")),
            in_folder("b", Some("new")),
            in_folder("c", Some("other")),
        ];
        let out = renamed_folder(&list, "old", "new", 5).unwrap();
        assert_eq!(out.len(), 3, "no connection is lost by a merge");
        assert_eq!(out[0].folder.as_deref(), Some("new"));
        assert_eq!(out[1].folder.as_deref(), Some("new"));
        assert_eq!(out[2].folder.as_deref(), Some("other"));
        assert_eq!(
            out[2].updated_at, None,
            "unaffected rows keep their timestamps"
        );
    }

    #[test]
    fn rename_to_a_blank_or_invalid_name_is_an_error() {
        let list = [in_folder("a", Some("old"))];
        assert!(renamed_folder(&list, "old", "  ", 5).is_err());
        assert!(renamed_folder(&list, "old", "a\nb", 5).is_err());
    }

    #[test]
    fn deleting_a_folder_moves_its_members_to_the_root_and_deletes_nothing() {
        let list = [in_folder("a", Some("gone")), in_folder("b", Some("kept"))];
        let out = deleted_folder(&list, "gone", 9);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].folder, None);
        assert_eq!(out[0].updated_at, Some(9));
        assert_eq!(out[1].folder.as_deref(), Some("kept"));
    }
}
