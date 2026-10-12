//! Locating widgets by their localized label.
use curlybrace_ui::I18n;
use curlybrace_ui::app::CurlybraceApp;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

/// Looks labels up from the real English locale so tests never hardcode UI text.
pub struct Labels(I18n);

impl Labels {
    pub fn new() -> Self {
        Self(I18n::english())
    }

    pub fn t(&self, key: &str) -> String {
        let text = self.0.t(key);
        assert!(!text.starts_with('⟦'), "missing i18n key {key}");
        text
    }

    pub fn t_with(&self, key: &str, params: &[(&str, &str)]) -> String {
        self.0.t_with(key, params)
    }

    pub fn i18n(&self) -> &I18n {
        &self.0
    }
}

pub type H<'a> = Harness<'a, CurlybraceApp>;

/// True when exactly-or-more nodes carry this label or value text.
pub fn shows(harness: &H<'_>, text: &str) -> bool {
    harness.query_all_by_label(text).next().is_some()
        || harness.query_all_by_value(text).next().is_some()
}

/// True when some label contains `part`.
pub fn shows_containing(harness: &H<'_>, part: &str) -> bool {
    harness.query_all_by_label_contains(part).next().is_some()
}

/// One line per visible accessibility node (`Role label|value`), for failure messages.
pub fn dump(harness: &H<'_>) -> String {
    harness
        .root()
        .children_recursive()
        .map(|n| n.accesskit_node())
        .filter(|n| n.role() != eframe::egui::accesskit::Role::TextRun)
        .map(|n| {
            let text = n.label().or_else(|| n.value()).unwrap_or_default();
            format!("{:?} {text:?}", n.role())
        })
        .collect::<Vec<_>>()
        .join("\n")
}
