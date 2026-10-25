//! The rule of a typed confirmation, shared by the Compare apply review and the paste dialog.

/// The typed text, trimmed, equals the required text exactly (case and all). A blank
/// requirement never passes, so a missing name cannot confirm anything.
pub fn typed_text_matches(required: &str, typed: &str) -> bool {
    let required = required.trim();
    !required.is_empty() && typed.trim() == required
}
