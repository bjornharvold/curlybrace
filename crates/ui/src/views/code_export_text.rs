//! The words of the Copy as code dialog that depend on what it shows: the key of each language's
//! name and of each variable field, and the message of a generation error (never any of the
//! request's content, only which option or type it is about).
use crate::I18n;
use crate::viewmodel::NameField;
use curlybrace_core::codegen::{CodegenError, Language};

/// The i18n key of a language's name (a product name, kept as a key for translators).
pub(super) fn language_label_key(language: Language) -> &'static str {
    match language {
        Language::Mongosh => "codegen.language.mongosh",
        Language::Node => "codegen.language.node",
        Language::Python => "codegen.language.python",
        Language::Java => "codegen.language.java",
        Language::CSharp => "codegen.language.csharp",
        Language::Go => "codegen.language.go",
        Language::Php => "codegen.language.php",
        Language::Ruby => "codegen.language.ruby",
        Language::Rust => "codegen.language.rust",
    }
}

/// The i18n key of a variable-name field's label.
pub(super) fn name_label_key(field: NameField) -> &'static str {
    match field {
        NameField::Client => "codegen.name.client",
        NameField::Db => "codegen.name.db",
        NameField::Collection => "codegen.name.collection",
    }
}

/// The field a `CodegenError::InvalidName` names (`client`, `db` or `collection`).
fn field_named(which: &str) -> NameField {
    match which {
        "client" => NameField::Client,
        "db" => NameField::Db,
        _ => NameField::Collection,
    }
}

/// The translated message of a generation error.
pub(super) fn error_text(error: &CodegenError, i18n: &I18n) -> String {
    match error {
        CodegenError::InvalidName(which) => {
            let field = i18n.t(name_label_key(field_named(which)));
            i18n.t_with("codegen.error.invalid_name", &[("field", &field)])
        }
        CodegenError::EmptyPipeline => i18n.t("codegen.error.empty_pipeline"),
        CodegenError::UnsupportedValue(kind) => {
            i18n.t_with("codegen.error.unsupported_value", &[("type", kind)])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_word_of_the_dialog_is_in_the_catalogue() {
        let i18n = I18n::english();
        let mut keys: Vec<&str> = Language::ALL.into_iter().map(language_label_key).collect();
        keys.extend([NameField::Client, NameField::Db, NameField::Collection].map(name_label_key));
        keys.extend([
            "codegen.button",
            "codegen.button_hint",
            "codegen.copy",
            "codegen.disabled.text_not_applied",
            "codegen.language.label",
            "codegen.note",
            "codegen.preview",
            "codegen.setup",
            "codegen.title",
            "codegen.write_stage",
        ]);
        for key in keys {
            assert!(!i18n.t(key).starts_with('\u{27e6}'), "{key} is missing");
        }
    }

    #[test]
    fn each_error_names_its_field_or_type_and_never_content() {
        let i18n = I18n::english();
        let name = error_text(&CodegenError::InvalidName("collection"), &i18n);
        assert!(name.contains(&i18n.t("codegen.name.collection")));
        assert!(!name.contains('{'), "{name}");
        assert_eq!(
            error_text(&CodegenError::EmptyPipeline, &i18n),
            i18n.t("codegen.error.empty_pipeline")
        );
        let kind = error_text(&CodegenError::UnsupportedValue("Symbol"), &i18n);
        assert!(kind.contains("Symbol") && !kind.contains('{'), "{kind}");
    }

    #[test]
    fn the_languages_keep_their_order() {
        let names: Vec<String> = Language::ALL
            .into_iter()
            .map(|l| I18n::english().t(language_label_key(l)))
            .collect();
        assert_eq!(
            names,
            [
                "mongosh", "Node.js", "Python", "Java", "C#", "Go", "PHP", "Ruby", "Rust"
            ]
        );
    }
}
