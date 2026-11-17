//! What the Copy as code dialog says about the aggregate's options: whether the code carries them
//! (the "Include options" check box), which ones, and whether the read preference was left out
//! because the code keeps a trailing `$out` / `$merge` (the confirmed run always reads on the
//! primary, so the code must not say otherwise).
use super::code_export::{CodeDialog, CodeExportUi};
use curlybrace_core::codegen::{CodeRequest, code_writes};
use curlybrace_core::pipeline_options::PipelineOptions;

/// What the dialog shows about the options.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeOptionsView {
    /// The check box.
    pub include: bool,
    /// The label keys of what the code sets (empty when the options are not included).
    pub labels: Vec<&'static str>,
    /// A read preference is set and the code leaves it out because it writes.
    pub read_preference_left_out: bool,
    /// The key of the check box's label: the aggregate's options, or the query's.
    pub include_key: &'static str,
}

/// `None` for a draft or a query with no option set (there is nothing to include).
pub fn code_options_view(dialog: &CodeDialog) -> Option<CodeOptionsView> {
    let (options, include_key) = match &dialog.request {
        CodeRequest::Aggregate { options, .. } => (options, "codegen.options.include_options"),
        CodeRequest::Find { query, .. } => (&query.options, "codegen.options.include_find_options"),
    };
    if options.is_empty() {
        return None;
    }
    let include = dialog.options.include_options;
    let left_out = include
        && options.read_preference.is_some()
        && code_writes(&dialog.request, &dialog.options);
    Some(CodeOptionsView {
        include,
        labels: if include {
            set_labels(options, left_out)
        } else {
            Vec::new()
        },
        read_preference_left_out: left_out,
        include_key,
    })
}

/// The label of each option the code sets, in the order of the Options section.
fn set_labels(options: &PipelineOptions, read_preference_left_out: bool) -> Vec<&'static str> {
    [
        (
            options.collation.is_some(),
            "aggregate.options.collation.enabled",
        ),
        (options.hint.is_some(), "aggregate.options.hint.label"),
        (
            options.allow_disk_use.is_some(),
            "aggregate.options.allow_disk_use.label",
        ),
        (
            options.max_time_ms.is_some(),
            "aggregate.options.max_time.label",
        ),
        (options.comment.is_some(), "aggregate.options.comment.label"),
        (
            options.read_preference.is_some() && !read_preference_left_out,
            "aggregate.options.read_preference.label",
        ),
    ]
    .into_iter()
    .filter_map(|(set, key)| set.then_some(key))
    .collect()
}

impl CodeExportUi {
    /// Whether the code carries the aggregate's options; remembered for the next dialog.
    pub fn include_options_set(self, include: bool) -> Self {
        self.options_edited(|options| curlybrace_core::codegen::CodeOptions {
            include_options: include,
            ..options
        })
    }
}

#[cfg(test)]
#[path = "code_export_options_tests.rs"]
mod tests;
