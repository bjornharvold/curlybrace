//! Flat-key translation layer backed by a nested JSON locale file.
use curlybrace_core::error::CoreError;
use serde_json::Value;
use std::collections::HashMap;

/// Translations keyed by dotted path (`"a.b.c"`).
#[derive(Debug, Clone)]
pub struct I18n {
    map: HashMap<String, String>,
}

impl I18n {
    /// Parses a nested JSON object of strings into flat dotted keys.
    pub fn from_json(src: &str) -> Result<Self, String> {
        let root: Value = serde_json::from_str(src).map_err(|e| e.to_string())?;
        if !root.is_object() {
            return Err("locale root must be a JSON object".to_string());
        }
        let mut map = HashMap::new();
        flatten("", &root, &mut map)?;
        Ok(Self { map })
    }

    /// The bundled English locale.
    pub fn english() -> Self {
        Self::from_json(include_str!("../locales/en.json"))
            .expect("bundled en.json must be a valid locale")
    }

    /// Translation for `key`, or a visible `⟦key⟧` marker when missing.
    pub fn t(&self, key: &str) -> String {
        self.map
            .get(key)
            .cloned()
            .unwrap_or_else(|| format!("⟦{key}⟧"))
    }

    /// Like [`t`](Self::t), replacing `{name}` placeholders in a single pass.
    /// Unknown placeholders are left intact; substituted values are not re-expanded.
    pub fn t_with(&self, key: &str, params: &[(&str, &str)]) -> String {
        let Some(template) = self.map.get(key) else {
            return self.t(key);
        };
        let mut out = String::with_capacity(template.len());
        let mut rest = template.as_str();
        while let Some(start) = rest.find('{') {
            out.push_str(&rest[..start]);
            let tail = &rest[start..];
            let Some(end) = tail.find('}') else {
                rest = tail;
                break;
            };
            let name = &tail[1..end];
            match params.iter().find(|(n, _)| *n == name) {
                Some((_, value)) => out.push_str(value),
                None => out.push_str(&tail[..=end]),
            }
            rest = &tail[end + 1..];
        }
        out.push_str(rest);
        out
    }

    /// User-facing text for a core error (technical detail is never shown).
    pub fn error(&self, e: &CoreError) -> String {
        self.t(e.i18n_key())
    }

    /// Headline for a failed runtime command context (e.g. `"connect"`).
    pub fn failure(&self, context: &str) -> String {
        self.t(&format!("failures.{context}"))
    }
}

fn flatten(prefix: &str, value: &Value, out: &mut HashMap<String, String>) -> Result<(), String> {
    match value {
        Value::String(text) => {
            out.insert(prefix.to_string(), text.clone());
            Ok(())
        }
        Value::Object(fields) => fields.iter().try_for_each(|(name, child)| {
            let path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}.{name}")
            };
            flatten(&path, child, out)
        }),
        _ => Err(format!("locale value at '{prefix}' must be a string")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every key the compare view-models name (the views' own labels are checked by the
    /// pseudo-locale frame tests in `app::compare_tests::frames`).
    const COMPARE_KEYS: [&str; 16] = [
        "compare.category.different",
        "compare.category.only_in_source",
        "compare.category.only_in_target",
        "compare.detail.added",
        "compare.detail.changed",
        "compare.detail.removed",
        "compare.preflight.empty_both",
        "compare.preflight.needs_index",
        "compare.setup.filter_invalid",
        "compare.setup.incomplete",
        "compare.setup.key_invalid",
        "compare.setup.rules_invalid",
        "compare.setup.same_collection",
        "compare.setup.view_selected",
        "compare.side.source",
        "compare.side.target",
    ];

    #[test]
    fn flattens_nested_objects() {
        let i = I18n::from_json(r#"{"a":{"b":"x","c":{"d":"y"}}}"#).unwrap();
        assert_eq!(i.t("a.b"), "x");
        assert_eq!(i.t("a.c.d"), "y");
    }

    #[test]
    fn t_with_replaces_params_and_keeps_unknown() {
        let i = I18n::from_json(r#"{"h":"hello {name}, {other}"}"#).unwrap();
        assert_eq!(i.t_with("h", &[("name", "Ann")]), "hello Ann, {other}");
    }

    #[test]
    fn t_with_is_single_pass() {
        let i = I18n::from_json(r#"{"h":"{a} {b}"}"#).unwrap();
        assert_eq!(i.t_with("h", &[("a", "{b}"), ("b", "Z")]), "{b} Z");
    }

    #[test]
    fn missing_key_is_marked() {
        let i = I18n::from_json("{}").unwrap();
        assert_eq!(i.t("nope.k"), "⟦nope.k⟧");
        assert_eq!(i.t_with("nope.k", &[("a", "b")]), "⟦nope.k⟧");
    }

    #[test]
    fn typed_core_errors_have_honest_copy() {
        let i = I18n::english();
        let tls = i.error(&CoreError::TlsThroughTunnel);
        assert!(tls.contains("invalid") && tls.contains("certificate protection"));
        assert!(i.error(&CoreError::SshHostKeyChanged).contains("changed"));
        assert!(!i.error(&CoreError::Superseded).starts_with('⟦'));
    }

    #[test]
    fn invalid_json_is_err() {
        assert!(I18n::from_json("{not json").is_err());
        assert!(I18n::from_json("[]").is_err());
    }

    #[test]
    fn non_string_leaves_are_rejected_with_key_path() {
        for src in [
            r#"{"a":{"n":1}}"#,
            r#"{"a":{"n":[1]}}"#,
            r#"{"a":{"n":true}}"#,
            r#"{"a":{"n":null}}"#,
        ] {
            let err = I18n::from_json(src).unwrap_err();
            assert!(err.contains("a.n"), "{err}");
        }
    }

    #[test]
    fn empty_objects_and_duplicates_do_not_panic() {
        let i = I18n::from_json(r#"{"a":{},"b":"x","b":{"c":"y"}}"#).unwrap();
        assert_eq!(i.t("b.c"), "y");
        assert_eq!(i.t("a"), "⟦a⟧");
    }

    #[test]
    fn english_covers_every_core_error_and_failure_context() {
        let i = I18n::english();
        let errors = [
            CoreError::InvalidUri(String::new()),
            CoreError::InvalidJson(String::new()),
            CoreError::InvalidDocument(String::new()),
            CoreError::Secret(String::new()),
            CoreError::Ssh(String::new()),
            CoreError::Unsupported(String::new()),
            CoreError::Database(String::new()),
            CoreError::NotFound,
            CoreError::Superseded,
            CoreError::AuthFailed,
            CoreError::Unauthorized,
            CoreError::SshHostKeyChanged,
            CoreError::TlsThroughTunnel,
            CoreError::CompareDuplicateKey,
            CoreError::CompareKeyMissing,
            CoreError::CompareKeyInvalid,
            CoreError::CompareKeyOrder,
            CoreError::CompareKeyNotIndexed,
            CoreError::CompareCapExceeded,
            CoreError::CompareSetupInvalid,
            CoreError::ApplyTargetNotWritable,
            CoreError::ApplyWriteRejected,
            CoreError::ApplyDuplicateKey,
            CoreError::ApplyOutcomeUnknown,
            CoreError::ExportWriteFailed,
        ];
        for e in &errors {
            let s = i.error(e);
            assert!(!s.starts_with('⟦'), "missing {}", e.i18n_key());
            assert!(!s.is_empty());
        }
        for c in [
            "connect",
            "list_databases",
            "list_collections",
            "find",
            "insert",
            "replace",
            "delete",
            "clone",
            "disconnect",
            "compare_open",
            "compare_databases",
            "compare_collections",
            "compare_check",
            "compare_start",
            "compare_rows",
            "compare_detail",
            "compare_duplicates",
            "compare_export",
            "compare_duplicates_timeout",
        ] {
            assert!(!i.failure(c).starts_with('⟦'), "missing failures.{c}");
        }
        for k in COMPARE_KEYS {
            assert!(!i.t(k).starts_with('⟦'), "missing {k}");
        }
        for k in [
            "app.title",
            "common.ok",
            "common.cancel",
            "common.save",
            "common.delete",
            "common.close",
            "common.retry",
        ] {
            assert!(!i.t(k).starts_with('⟦'), "missing {k}");
        }
    }
}
