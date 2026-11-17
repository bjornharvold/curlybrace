//! Rust driver (`mongodb` 3, async) renderer.

use bson::Document;

use crate::codegen::Renderer;
use std::collections::BTreeSet;

use crate::codegen::agg_options::{
    CodeAggOptions, alternate_words, case_first_words, mode_words, strength_words,
};
use crate::codegen::escape::indent_lines;
use crate::codegen::lossy::with_note;
use crate::codegen::model::{CodeOptions, CodegenError};
use crate::codegen::range::i64_arg;
use crate::codegen::rust_syntax::{RustSyntax, quote};
use crate::codegen::walk::{render_document, render_documents};
use crate::pipeline_options::{CollationOption, HintOption, ReadPreferenceMode};
use crate::store::{Namespace, Query};

/// Longest single-line `use` before rustfmt would wrap the name list.
const IMPORT_WIDTH: usize = 100;

pub(crate) struct Rust;

impl Rust {
    fn frame(
        ns: &Namespace,
        options: &CodeOptions,
        syn: &RustSyntax,
        body: &str,
        used: &Used,
    ) -> String {
        let limit = syn.recursion_limit();
        let noted = with_note(syn, "//", body.to_string());
        let body = noted.as_str();
        if !options.setup {
            return match limit {
                // The attribute is only legal at the crate root.
                Some(n) => {
                    format!("// Needs #![recursion_limit = \"{n}\"] at the top of main.rs\n{body}")
                }
                None => noted,
            };
        }
        let attribute = limit
            .map(|n| format!("#![recursion_limit = \"{n}\"]\n\n"))
            .unwrap_or_default();
        let bson_import = if syn.uses_bson_path() {
            "{self, Document, doc}"
        } else {
            "{Document, doc}"
        };
        let names: Vec<&str> = used.names.iter().copied().collect();
        let option_imports = options_import(&names);
        let duration = if used.duration {
            "use std::time::Duration;\n"
        } else {
            ""
        };
        let (client, db, coll) = (
            &options.client_var,
            &options.db_var,
            &options.collection_var,
        );
        let inner = format!(
            "let {client} = Client::with_uri_str(\"<your connection string>\").await?;\nlet {db} = {client}.database({});\nlet {coll} = {db}.collection::<Document>({});\n\n{}\n\nOk(())",
            quote(&ns.db),
            quote(&ns.collection),
            body.trim_end(),
        );
        format!(
            "{attribute}use futures::TryStreamExt;\nuse mongodb::Client;\nuse mongodb::bson::{bson_import};\n{option_imports}{duration}\n#[tokio::main]\nasync fn main() -> Result<(), Box<dyn std::error::Error>> {{\n{}\n}}\n",
            indent_lines(&inner, "    ", 1),
        )
    }
}

const PRINT_CURSOR: &str =
    "while let Some(item) = cursor.try_next().await? {\n    println!(\"{item}\");\n}\n";

impl Renderer for Rust {
    fn find(
        &self,
        ns: &Namespace,
        query: &Query,
        _agg: &CodeAggOptions,
        options: &CodeOptions,
    ) -> Result<String, CodegenError> {
        let syn = RustSyntax::new();
        let mut body = format!(
            "let filter = doc! {};\n",
            render_document(&syn, &query.filter, 0)?
        );
        let mut chain: Vec<String> = Vec::new();
        if let Some(projection) = &query.projection {
            body.push_str(&format!(
                "let projection = doc! {};\n",
                render_document(&syn, projection, 0)?
            ));
            chain.push(".projection(projection)".into());
        }
        if let Some(sort) = &query.sort {
            body.push_str(&format!(
                "let sort = doc! {};\n",
                render_document(&syn, sort, 0)?
            ));
            chain.push(".sort(sort)".into());
        }
        if query.skip > 0 {
            chain.push(format!(".skip({})", i64_arg("skip", query.skip)?));
        }
        if query.limit > 0 {
            chain.push(format!(".limit({})", i64_arg("limit", query.limit)?));
        }
        let lines: String = chain.iter().map(|c| format!("\n    {c}")).collect();
        body.push_str(&format!(
            "\nlet mut cursor = {}\n    .find(filter){lines}\n    .await?;\n{PRINT_CURSOR}",
            options.collection_var
        ));
        Ok(Self::frame(ns, options, &syn, &body, &Used::default()))
    }

    fn aggregate(
        &self,
        ns: &Namespace,
        stages: &[Document],
        agg: &CodeAggOptions,
        options: &CodeOptions,
    ) -> Result<String, CodegenError> {
        let syn = RustSyntax::new();
        let pipeline = render_documents(&syn, stages, 0)?;
        let mut used = Used::default();
        let setters = Self::option_setters(&syn, agg, &mut used)?;
        let coll = &options.collection_var;
        let call = if setters.is_empty() {
            format!("{coll}.aggregate(pipeline).await?;")
        } else {
            let lines: String = setters.iter().map(|s| format!("\n    {s}")).collect();
            format!("{coll}\n    .aggregate(pipeline){lines}\n    .await?;")
        };
        let body = format!("let pipeline = {pipeline};\n\nlet mut cursor = {call}\n{PRINT_CURSOR}");
        let body = agg.with_note("//", body);
        Ok(Self::frame(ns, options, &syn, &body, &used))
    }
}

/// What the option setters name, so the frame can import it.
#[derive(Default)]
struct Used {
    /// Names under `mongodb::options`.
    names: BTreeSet<&'static str>,
    duration: bool,
}

impl Rust {
    /// The setters on the `aggregate` action, in the order collation, hint, allowDiskUse,
    /// maxTimeMS, comment, read preference; a multi-line setter carries its own indent.
    fn option_setters(
        syn: &RustSyntax,
        agg: &CodeAggOptions,
        used: &mut Used,
    ) -> Result<Vec<String>, CodegenError> {
        let mut setters = Vec::new();
        if let Some(collation) = &agg.collation {
            used.names.insert("Collation");
            setters.push(Self::collation(collation, used));
        }
        match &agg.hint {
            Some(HintOption::Name(name)) => {
                used.names.insert("Hint");
                setters.push(format!(".hint(Hint::Name({}.to_string()))", quote(name)));
            }
            Some(HintOption::Keys(keys)) => {
                used.names.insert("Hint");
                let keys = render_document(syn, keys, 1)?;
                setters.push(format!(".hint(Hint::Keys(doc! {keys}))"));
            }
            None => {}
        }
        if let Some(on) = agg.allow_disk_use {
            setters.push(format!(".allow_disk_use({on})"));
        }
        if let Some(ms) = agg.max_time_ms {
            used.duration = true;
            setters.push(format!(".max_time(Duration::from_millis({ms}))"));
        }
        if let Some(comment) = &agg.comment {
            setters.push(format!(".comment({})", quote(comment)));
        }
        if let Some(mode) = agg.read_preference {
            used.names.insert("ReadPreference");
            used.names.insert("SelectionCriteria");
            let preference = match mode {
                ReadPreferenceMode::Primary => "ReadPreference::Primary".to_string(),
                other => format!(
                    "ReadPreference::{} {{ options: None }}",
                    mode_words(other).pascal
                ),
            };
            setters.push(format!(
                ".selection_criteria(SelectionCriteria::ReadPreference(\n        {preference},\n    ))"
            ));
        }
        Ok(setters)
    }

    /// `.collation(Collation::builder()...build())`, one setter per line when there are several.
    fn collation(collation: &CollationOption, used: &mut Used) -> String {
        let mut setters = vec![format!(".locale({})", quote(&collation.locale))];
        if let Some(strength) = collation.strength {
            used.names.insert("CollationStrength");
            let (word, _) = strength_words(strength);
            setters.push(format!(".strength(CollationStrength::{word})"));
        }
        if let Some(on) = collation.case_level {
            setters.push(format!(".case_level({on})"));
        }
        if let Some(case_first) = collation.case_first {
            used.names.insert("CollationCaseFirst");
            let (word, _) = case_first_words(case_first);
            setters.push(format!(".case_first(CollationCaseFirst::{word})"));
        }
        if let Some(on) = collation.numeric_ordering {
            setters.push(format!(".numeric_ordering({on})"));
        }
        if let Some(alternate) = collation.alternate {
            used.names.insert("CollationAlternate");
            let (word, _) = alternate_words(alternate);
            setters.push(format!(".alternate(CollationAlternate::{word})"));
        }
        if setters.len() == 1 {
            return format!(".collation(Collation::builder(){}.build())", setters[0]);
        }
        let lines: String = setters
            .iter()
            .map(|s| format!("\n            {s}"))
            .collect();
        format!(".collation(\n        Collation::builder(){lines}\n            .build(),\n    )")
    }
}

/// `use mongodb::options::{..};` for `names` (nothing without names): on one line while that
/// line fits the width (the newline does not count), else in rustfmt's wrapped form with the
/// names packed into lines up to the width.
fn options_import(names: &[&str]) -> String {
    if names.is_empty() {
        return String::new();
    }
    let single = format!("use mongodb::options::{{{}}};", names.join(", "));
    if single.len() <= IMPORT_WIDTH {
        return format!("{single}\n");
    }
    let mut lines = String::new();
    let mut line = String::new();
    for name in names {
        let piece = format!("{name},");
        if !line.is_empty() && 4 + line.len() + 1 + piece.len() > IMPORT_WIDTH {
            lines.push_str(&format!("    {line}\n"));
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(&piece);
    }
    lines.push_str(&format!("    {line}\n"));
    format!("use mongodb::options::{{\n{lines}}};\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `count` names of `len` characters, so the single line has a known width.
    fn names_of(len: usize, count: usize) -> Vec<String> {
        (0..count)
            .map(|i| format!("{i:0>width$}", width = len))
            .collect()
    }

    #[test]
    fn a_use_line_of_exactly_the_width_stays_on_one_line() {
        // "use mongodb::options::{" and "};" are 25 characters; one name fills the other 75.
        let name = "N".repeat(IMPORT_WIDTH - 25);
        let text = options_import(&[name.as_str()]);
        assert_eq!(text.trim_end().len(), IMPORT_WIDTH);
        assert_eq!(text.lines().count(), 1, "{text}");
        let over = "N".repeat(IMPORT_WIDTH - 24);
        assert!(options_import(&[over.as_str()]).lines().count() > 1);
    }

    #[test]
    fn a_longer_import_wraps_into_lines_of_at_most_the_width() {
        let owned = names_of(20, 8);
        let names: Vec<&str> = owned.iter().map(String::as_str).collect();
        let text = options_import(&names);
        assert!(text.starts_with("use mongodb::options::{\n"), "{text}");
        assert!(
            text.lines().all(|line| line.len() <= IMPORT_WIDTH),
            "{text}"
        );
    }

    #[test]
    fn no_names_no_import() {
        assert_eq!(options_import(&[]), "");
    }
}
