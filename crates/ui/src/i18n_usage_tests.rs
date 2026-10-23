//! Keeps `compare.*` / `workspace.*` keys and the code that uses them in step: every key in
//! `en.json` is referenced by some source file, and every key a source file names exists.
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const PREFIXES: [&str; 4] = ["compare.", "connection.", "connections.", "workspace."];
/// Keys outside the prefixes that the Compare feature added.
const EXTRA_KEYS: [&str; 2] = ["sidebar.compare_database_to", "sidebar.compare_to"];

/// Literals that look like a key under a tracked prefix but are something else (file names).
const NOT_KEYS: [&str; 1] = ["connections.json"];

fn leaves(prefix: &str, value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(fields) => {
            for (name, child) in fields {
                let path = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}.{name}")
                };
                leaves(&path, child, out);
            }
        }
        _ => {
            out.insert(prefix.to_string());
        }
    }
}

fn catalogue() -> BTreeSet<String> {
    let root: Value = serde_json::from_str(include_str!("../locales/en.json")).unwrap();
    let mut keys = BTreeSet::new();
    leaves("", &root, &mut keys);
    keys
}

fn is_tracked(key: &str) -> bool {
    PREFIXES.iter().any(|p| key.starts_with(p)) || EXTRA_KEYS.contains(&key)
}

/// Production sources only: tests name keys on purpose and would hide an unused one.
fn source_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if path.is_dir() {
            if !name.ends_with("_tests") {
                source_files(&path, out);
            }
        } else if name.ends_with(".rs") && !name.ends_with("_tests.rs") && name != "i18n.rs" {
            out.push(path);
        }
    }
}

/// The code of a file that ships: comment lines are dropped, and everything from the first
/// `#[cfg(test)]` on is cut. Convention: an inline test module is the LAST item of its file.
fn production_text(src: &str) -> String {
    let code = src.split("#[cfg(test)]").next().unwrap_or_default();
    code.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every string literal in `src` that looks like a tracked key.
fn literals(src: &str) -> Vec<String> {
    src.split('"')
        .skip(1)
        .step_by(2)
        .filter(|s| is_key_shaped(s) && is_tracked(s) && !NOT_KEYS.contains(s))
        .map(str::to_string)
        .collect()
}

fn is_key_shaped(s: &str) -> bool {
    s.contains('.')
        && s.split('.').all(|part| {
            !part.is_empty() && part.chars().all(|c| c.is_ascii_lowercase() || c == '_')
        })
}

fn referenced() -> BTreeSet<String> {
    let mut files = Vec::new();
    source_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    // Core returns i18n keys for the UI (as string literals) from these production files.
    files.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../core/src/compare/apply_fields.rs"));
    // Test code is either in `*_tests.rs` files (skipped above) or in an inline `#[cfg(test)]`
    // module, which `production_text` cuts off, so what is left is production code.
    files
        .iter()
        .flat_map(|f| literals(&production_text(&std::fs::read_to_string(f).unwrap())))
        .collect()
}

#[test]
fn every_tracked_catalogue_key_is_used_by_code() {
    let used = referenced();
    let unused: Vec<_> = catalogue()
        .into_iter()
        .filter(|k| is_tracked(k) && !used.contains(k))
        .collect();
    assert!(unused.is_empty(), "keys no code uses: {unused:?}");
}

#[test]
fn every_key_the_code_names_exists_in_the_catalogue() {
    let known = catalogue();
    let missing: Vec<_> = referenced()
        .into_iter()
        .filter(|k| !known.contains(k))
        .collect();
    assert!(missing.is_empty(), "keys missing from en.json: {missing:?}");
}

#[test]
fn comments_and_trailing_test_modules_do_not_count_as_use() {
    let src = "let a = t(\"compare.run.start\");\n// \"compare.run.cancel\"\n#[cfg(test)]\nmod t { \"compare.run.discard\" }";
    assert_eq!(literals(&production_text(src)), vec!["compare.run.start"]);
}

#[test]
fn the_scanner_recognises_keys_and_ignores_other_text() {
    let found = literals(r#"t("compare.run.start"); let x = "a.b"; "compare.Bad.Key"; "1.5""#);
    assert_eq!(found, vec!["compare.run.start".to_string()]);
}

/// The object keys of `json` in file order with their nesting depth (2-space indentation).
fn keys_in_file_order(json: &str) -> Vec<(usize, String)> {
    json.lines()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            let rest = trimmed.strip_prefix('"')?;
            let (key, after) = rest.split_once('"')?;
            after
                .starts_with(':')
                .then(|| ((line.len() - trimmed.len()) / 2, key.to_string()))
        })
        .collect()
}

/// The first key that is not after its previous sibling, if any.
fn first_disorder(json: &str) -> Option<String> {
    let mut last: Vec<Option<String>> = Vec::new();
    for (depth, key) in keys_in_file_order(json) {
        last.truncate(depth + 1);
        last.resize(depth + 1, None);
        if last[depth]
            .as_ref()
            .is_some_and(|previous| *previous >= key)
        {
            return Some(key);
        }
        last[depth] = Some(key);
    }
    None
}

#[test]
fn en_json_is_nested_and_alphabetical_at_every_level() {
    let disorder = first_disorder(include_str!("../locales/en.json"));
    assert_eq!(disorder, None, "key out of order");
}

#[test]
fn the_order_check_notices_an_out_of_order_key() {
    let json = "{\n  \"b\": {\n    \"y\": \"1\",\n    \"x\": \"2\"\n  }\n}";
    assert_eq!(first_disorder(json), Some("x".to_string()));
    let fine = "{\n  \"b\": {\n    \"x\": \"1\",\n    \"y\": \"2\"\n  },\n  \"c\": {\n    \"a\": \"3\"\n  }\n}";
    assert_eq!(first_disorder(fine), None);
}

#[test]
fn host_hints_name_every_character_a_host_may_hold() {
    let root: Value = serde_json::from_str(include_str!("../locales/en.json")).unwrap();
    for key in ["invalid_uri_bad_host", "invalid_uri_non_ascii_host"] {
        let hint = root["errors"][key].as_str().unwrap();
        assert!(hint.contains("underscores"), "{key}: {hint}");
    }
}

#[test]
fn the_bad_host_hint_covers_every_kind_of_bad_host() {
    let root: Value = serde_json::from_str(include_str!("../locales/en.json")).unwrap();
    let hint = root["errors"]["invalid_uri_bad_host"].as_str().unwrap();
    for word in ["Unicode", "IPv6", "socket"] {
        assert!(hint.contains(word), "{word} missing: {hint}");
    }
}

#[test]
fn the_non_ascii_hint_names_the_button_by_its_current_label() {
    let root: Value = serde_json::from_str(include_str!("../locales/en.json")).unwrap();
    let hint = root["errors"]["invalid_uri_non_ascii_host"]
        .as_str()
        .unwrap();
    let label = root["connections"]["from_uri"].as_str().unwrap();
    assert!(hint.contains(label), "{hint}");
}

#[test]
fn the_secret_hint_covers_the_password_and_the_certificate_and_proxy_secrets() {
    let root: Value = serde_json::from_str(include_str!("../locales/en.json")).unwrap();
    let hint = root["errors"]["invalid_uri_secret"].as_str().unwrap();
    for word in ["passphrase", "certificate", "proxy", "password field"] {
        assert!(hint.contains(word), "{word} missing: {hint}");
    }
}
