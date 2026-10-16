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
    uri_host_list(uri).join(", ")
}

/// The hosts of a mongodb URI one by one (see [`uri_hosts`]); empty when there are none.
pub fn uri_host_list(uri: &str) -> Vec<String> {
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
        return Vec::new();
    }
    hosts.split(',').map(str::to_owned).collect()
}

/// Characters that separate words: control whitespace plus the Unicode line breaks (NEL, line
/// and paragraph separator). Each run of them becomes one space rather than vanishing, so
/// "a\nb" reads "a b" and not "ab". `validate()` still rejects them in new input.
fn is_word_separator(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

/// `text` with each run of separators turned into one space and every other forbidden
/// character removed, trimmed, cut to `max` chars (never mid-char), trimmed. Spaces already in
/// the text are never collapsed: the minimal change that keeps words apart.
fn clean(text: &str, max: usize) -> String {
    let mut spaced = String::with_capacity(text.len());
    let mut in_run = false;
    for c in text.chars() {
        if is_word_separator(c) {
            if !in_run {
                spaced.push(' ');
            }
            in_run = true;
        } else if !is_forbidden_label_char(c) {
            spaced.push(c);
            in_run = false;
        }
    }
    let cut: String = spaced.trim().chars().take(max).collect();
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
/// Each label is repaired only if it fails its own check; one that already passes is left
/// exactly as saved. A failing label is cleaned: control whitespace between words (newline,
/// tab, ...) becomes a space, other control and invisible format characters are removed, it is
/// trimmed and cut to its limit, a name left blank falls back to the URI host (then
/// `connection-<id prefix>`) and an emptied folder becomes `None`. Everything else, including
/// the id, is untouched.
pub fn repair_labels(cfg: ConnectionConfig) -> (ConnectionConfig, bool) {
    let name = if cfg.name_is_valid() {
        cfg.name.clone()
    } else {
        let cleaned = clean(&cfg.name, MAX_NAME_LEN);
        if cleaned.is_empty() {
            fallback_name(&cfg)
        } else {
            cleaned
        }
    };
    let folder = if cfg.folder_is_valid() {
        cfg.folder.clone()
    } else {
        cfg.folder
            .as_deref()
            .map(|f| clean(f, MAX_FOLDER_LEN))
            .filter(|f| !f.is_empty())
    };
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
        c.folder = Some("wo\u{0}rk\u{200B}".into());
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

    #[test]
    fn control_whitespace_between_words_becomes_a_space() {
        let mut c = cfg("a\nb\rc\td", "mongodb://h");
        c.folder = Some("x\ny".into());
        let (out, changed) = repair_labels(c);
        assert_eq!(out.name, "a b c d");
        assert_eq!(out.folder.as_deref(), Some("x y"));
        assert!(changed);
        assert!(out.validate().is_ok());
    }

    #[test]
    fn other_forbidden_characters_are_still_removed_and_spaces_are_not_collapsed() {
        // Only the separating control whitespace turns into a space; zero-width and other
        // control characters vanish, and spaces already there are left as they are.
        let (out, _) = repair_labels(cfg("a\u{200B}b\u{0}c  d\ne", "mongodb://h"));
        assert_eq!(out.name, "abc  d e");
    }

    #[test]
    fn a_separator_at_the_edges_is_trimmed_away() {
        let (out, _) = repair_labels(cfg("\nab\t", "mongodb://h"));
        assert_eq!(out.name, "ab");
    }

    #[test]
    fn a_run_of_separators_becomes_one_space_in_name_and_folder() {
        for sep in [
            "\r\n",
            "\n\n",
            "\u{2028}",
            "\u{85}",
            "\u{2029}",
            "\t\r\n\u{2028}",
        ] {
            let mut c = cfg(&format!("a{sep}b"), "mongodb://h");
            c.folder = Some(format!("a{sep}b"));
            let (out, changed) = repair_labels(c);
            assert_eq!(out.name, "a b", "name for {sep:?}");
            assert_eq!(out.folder.as_deref(), Some("a b"), "folder for {sep:?}");
            assert!(changed);
            assert!(out.validate().is_ok());
        }
    }

    #[test]
    fn a_control_character_inside_a_word_is_still_removed() {
        let (out, _) = repair_labels(cfg("wo\u{0}rk", "mongodb://h"));
        assert_eq!(out.name, "work");
    }

    #[test]
    fn a_leading_or_trailing_separator_run_is_trimmed_away() {
        let (out, _) = repair_labels(cfg("\r\n\u{2028}ab\u{85}\u{2029}", "mongodb://h"));
        assert_eq!(out.name, "ab");
    }

    #[test]
    fn a_label_that_already_passes_its_own_check_is_left_as_saved() {
        // The name is valid (edge spaces are allowed); only the folder fails its check.
        let mut c = cfg("prod ", "mongodb://h");
        c.folder = Some(" work ".into());
        let (out, changed) = repair_labels(c);
        assert_eq!(out.name, "prod ");
        assert_eq!(out.folder.as_deref(), Some("work"));
        assert!(changed);
        // And the other way round: a valid folder stays while the name is repaired.
        let mut c = cfg("a\nb", "mongodb://h");
        c.folder = Some("work".into());
        let (out, changed) = repair_labels(c);
        assert_eq!(out.name, "a b");
        assert_eq!(out.folder.as_deref(), Some("work"));
        assert!(changed);
    }

    #[test]
    fn nothing_changes_when_both_labels_pass() {
        let mut c = cfg(" prod ", "mongodb://h");
        c.folder = Some("work".into());
        let (out, changed) = repair_labels(c.clone());
        assert_eq!(out, c);
        assert!(!changed);
    }

    #[test]
    fn validation_still_rejects_the_line_and_paragraph_separators() {
        assert!(cfg("a\u{2028}b", "mongodb://h").validate().is_err());
        assert!(cfg("a\u{2029}b", "mongodb://h").validate().is_err());
    }
}
