//! Repairs the labels (name, folder) of a connection read from the user's own saved file.
//!
//! Releases before the label rules accepted any non-blank name, so an upgraded user may have
//! saved connections that the strict `validate()` now refuses. Dropping them would orphan their
//! keychain secrets, so the LOAD path repairs the labels instead. New input (the edit dialog,
//! import) stays strict and never goes through here.
use crate::connection::{ConnectionConfig, MAX_FOLDER_LEN, MAX_NAME_LEN, is_forbidden_label_char};

/// The host part of a mongodb URI, without credentials, path or options; several hosts are
/// joined with ", ". Never panics; a text that is not a URI yields whatever host-like text it
/// has (possibly empty).
pub fn uri_hosts(uri: &str) -> String {
    let rest = uri
        .strip_prefix("mongodb+srv://")
        .or_else(|| uri.strip_prefix("mongodb://"))
        .unwrap_or(uri);
    // Options first (they may hold '@' or '/'), then everything up to the last '@' is
    // userinfo (even if it holds a raw '/'), then the path.
    let before_options = rest.split('?').next().unwrap_or("");
    let after_userinfo = before_options
        .rsplit_once('@')
        .map_or(before_options, |(_, h)| h);
    let hosts = after_userinfo.split('/').next().unwrap_or("");
    if hosts.is_empty() {
        return String::new();
    }
    hosts.split(',').collect::<Vec<_>>().join(", ")
}

/// `text` without forbidden characters, trimmed, cut to `max` chars (never mid-char), trimmed.
fn clean(text: &str, max: usize) -> String {
    let stripped: String = text
        .chars()
        .filter(|c| !is_forbidden_label_char(*c))
        .collect();
    let cut: String = stripped.trim().chars().take(max).collect();
    cut.trim().to_owned()
}

fn fallback_name(cfg: &ConnectionConfig) -> String {
    let host = clean(&uri_hosts(&cfg.uri), MAX_NAME_LEN);
    if host.is_empty() {
        let prefix: String = cfg.id.simple().to_string().chars().take(8).collect();
        format!("connection-{prefix}")
    } else {
        host
    }
}

/// `cfg` with a name and folder that satisfy the label rules, and whether anything changed.
/// Control and invisible format characters are removed, both labels are trimmed and cut to
/// their limits, a name left blank falls back to the URI host (then `connection-<id prefix>`)
/// and an emptied folder becomes `None`. Everything else, including the id, is untouched.
pub fn repair_labels(cfg: ConnectionConfig) -> (ConnectionConfig, bool) {
    let mut name = clean(&cfg.name, MAX_NAME_LEN);
    if name.is_empty() {
        name = fallback_name(&cfg);
    }
    let folder = cfg
        .folder
        .as_deref()
        .map(|f| clean(f, MAX_FOLDER_LEN))
        .filter(|f| !f.is_empty());
    let changed = name != cfg.name || folder != cfg.folder;
    (
        ConnectionConfig {
            name,
            folder,
            ..cfg
        },
        changed,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(name: &str, uri: &str) -> ConnectionConfig {
        ConnectionConfig::new(name, uri)
    }

    #[test]
    fn a_valid_connection_is_returned_unchanged() {
        let mut c = cfg("Prod \u{1F680} データ", "mongodb://h");
        c.folder = Some("work".into());
        let (out, changed) = repair_labels(c.clone());
        assert_eq!(out, c);
        assert!(!changed);
    }

    #[test]
    fn invisible_and_control_characters_are_removed_from_both_labels() {
        let mut c = cfg("pr\u{200D}o\u{202E}d\n", "mongodb://h");
        c.folder = Some("wo\nrk\u{200B}".into());
        let (out, changed) = repair_labels(c);
        assert_eq!(out.name, "prod");
        assert_eq!(out.folder.as_deref(), Some("work"));
        assert!(changed);
        assert!(out.validate().is_ok());
    }

    #[test]
    fn labels_are_trimmed_and_cut_to_their_limits_on_char_boundaries() {
        let long = format!("  {}  ", "é".repeat(MAX_NAME_LEN + 50));
        let mut c = cfg(&long, "mongodb://h");
        c.folder = Some("f".repeat(MAX_FOLDER_LEN + 5));
        let (out, changed) = repair_labels(c);
        assert_eq!(out.name.chars().count(), MAX_NAME_LEN);
        assert_eq!(
            out.folder.as_deref().map(|f| f.chars().count()),
            Some(MAX_FOLDER_LEN)
        );
        assert!(changed);
        assert!(out.validate().is_ok());
    }

    #[test]
    fn a_cut_never_leaves_trailing_whitespace() {
        let c = cfg(
            &format!("{} tail", "a".repeat(MAX_NAME_LEN - 1)),
            "mongodb://h",
        );
        let (out, _) = repair_labels(c);
        assert_eq!(out.name, "a".repeat(MAX_NAME_LEN - 1));
        assert!(out.validate().is_ok());
    }

    #[test]
    fn a_blank_name_falls_back_to_the_uri_host_without_credentials() {
        let (out, changed) = repair_labels(cfg("\u{0}\u{200D}", "mongodb://me@h1:1,h2:2/db?x=1"));
        assert_eq!(out.name, "h1:1, h2:2");
        assert!(changed);
    }

    #[test]
    fn a_blank_name_with_no_host_falls_back_to_the_id_prefix() {
        let c = cfg("  ", "mongodb://");
        let expected = format!("connection-{}", &c.id.simple().to_string()[..8]);
        let (out, _) = repair_labels(c);
        assert_eq!(out.name, expected);
    }

    #[test]
    fn an_emptied_folder_becomes_none_and_the_id_is_kept() {
        let mut c = cfg("a", "mongodb://h");
        c.folder = Some(" \n ".into());
        let id = c.id;
        let (out, changed) = repair_labels(c);
        assert_eq!(out.folder, None);
        assert_eq!(out.id, id);
        assert!(changed);
    }
}
