//! Editor modal state: validation runs before a Save action can be produced.
use bson::{Bson, Document};
use mongostudio_core::crud::id_of;
use mongostudio_core::ejson::{doc_to_editor_json, editor_json_to_doc};
use mongostudio_core::error::CoreError;

#[derive(Debug, Clone, PartialEq)]
pub struct EditorState {
    /// `_id` of the document being edited; `None` for a new document.
    pub original_id: Option<Bson>,
    pub text: String,
    /// The text the editor was opened with; a modal dismissal only discards unchanged text.
    pub initial_text: String,
    pub error: Option<CoreError>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EditorAction {
    Save {
        original_id: Option<Bson>,
        text: String,
    },
    Cancel,
}

/// What the user did in the modal this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorIntent {
    Save,
    Cancel,
}

impl EditorState {
    pub fn for_edit(doc: &Document) -> Self {
        let text = doc_to_editor_json(doc);
        Self {
            original_id: id_of(doc).ok().cloned(),
            initial_text: text.clone(),
            text,
            error: None,
        }
    }

    pub fn for_new() -> Self {
        Self {
            original_id: None,
            text: "{}".to_string(),
            initial_text: "{}".to_string(),
            error: None,
        }
    }

    fn with_error(self, error: CoreError) -> Self {
        Self {
            error: Some(error),
            ..self
        }
    }
}

/// What a modal dismissal (click outside, Escape) does: cancel only an untouched editor, so
/// unsaved edits are never thrown away by a stray click. The explicit Cancel button is not
/// affected (it goes through [`resolve_editor`] with `EditorIntent::Cancel`).
pub fn dismissal_action(state: &EditorState) -> Option<EditorAction> {
    (state.text == state.initial_text).then_some(EditorAction::Cancel)
}

/// Validates the text; on failure returns the state with `error` set (text untouched).
#[allow(clippy::result_large_err)] // the signature returns the whole next state by design
pub fn try_save(state: &EditorState) -> Result<EditorAction, EditorState> {
    match editor_json_to_doc(&state.text) {
        Ok(_) => Ok(EditorAction::Save {
            original_id: state.original_id.clone(),
            text: state.text.clone(),
        }),
        Err(e) => Err(state.clone().with_error(e)),
    }
}

/// Resolves a user intent into the next state and the action to emit, if any.
pub fn resolve_editor(
    state: &EditorState,
    intent: EditorIntent,
) -> (EditorState, Option<EditorAction>) {
    match intent {
        EditorIntent::Cancel => (state.clone(), Some(EditorAction::Cancel)),
        EditorIntent::Save => match try_save(state) {
            Ok(action) => (state.clone(), Some(action)),
            Err(next) => (next, None),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::doc;
    use mongostudio_core::test_support::mock_objects::{exotic_doc, mock_doc_with_id};

    #[test]
    fn for_new_is_empty_object_without_id() {
        let s = EditorState::for_new();
        assert_eq!(
            (s.text.as_str(), s.original_id, s.error),
            ("{}", None, None)
        );
    }

    #[test]
    fn for_edit_text_parses_back_to_same_document() {
        let d = mock_doc_with_id(3_i32);
        let s = EditorState::for_edit(&d);
        assert_eq!(s.original_id, Some(Bson::Int32(3)));
        assert_eq!(editor_json_to_doc(&s.text).unwrap(), d);
    }

    #[test]
    fn exotic_document_survives_edit_then_save() {
        let d = exotic_doc();
        let s = EditorState::for_edit(&d);
        let Ok(EditorAction::Save { original_id, text }) = try_save(&s) else {
            panic!("expected save");
        };
        assert_eq!(original_id, Some(d.get("_id").unwrap().clone()));
        let back = editor_json_to_doc(&text).unwrap();
        assert_eq!(
            back.keys().collect::<Vec<_>>(),
            d.keys().collect::<Vec<_>>()
        );
        for (k, v) in &d {
            match v {
                Bson::Double(x) if x.is_nan() => assert!(back.get_f64(k).unwrap().is_nan()),
                _ => assert_eq!(back.get(k), Some(v), "{k}"),
            }
        }
    }

    #[test]
    fn invalid_json_sets_error_keeps_text_and_emits_nothing() {
        let s = EditorState {
            text: "{ nope".into(),
            ..EditorState::for_new()
        };
        let Err(next) = try_save(&s) else {
            panic!("expected failure");
        };
        assert!(matches!(next.error, Some(CoreError::InvalidJson(_))));
        assert_eq!(next.text, "{ nope");
        let (state, action) = resolve_editor(&s, EditorIntent::Save);
        assert_eq!(action, None);
        assert!(state.error.is_some());
    }

    #[test]
    fn non_object_is_invalid_document() {
        let s = EditorState {
            text: "[1]".into(),
            ..EditorState::for_new()
        };
        assert!(matches!(
            try_save(&s),
            Err(EditorState {
                error: Some(CoreError::InvalidDocument(_)),
                ..
            })
        ));
    }

    #[test]
    fn cancel_always_emits_cancel_and_new_doc_saves_without_id() {
        let s = EditorState::for_new();
        assert_eq!(
            resolve_editor(&s, EditorIntent::Cancel).1,
            Some(EditorAction::Cancel)
        );
        let s = EditorState {
            text: doc_to_editor_json(&doc! { "a": 1_i32 }),
            ..s
        };
        assert!(matches!(
            resolve_editor(&s, EditorIntent::Save).1,
            Some(EditorAction::Save {
                original_id: None,
                ..
            })
        ));
    }

    #[test]
    fn dismissing_an_untouched_editor_cancels() {
        for state in [EditorState::for_new(), EditorState::for_edit(&exotic_doc())] {
            assert_eq!(dismissal_action(&state), Some(EditorAction::Cancel));
        }
    }

    #[test]
    fn dismissing_an_editor_with_unsaved_changes_is_ignored() {
        let mut state = EditorState::for_edit(&mock_doc_with_id(1_i32));
        state.text.push(' ');
        assert_eq!(dismissal_action(&state), None);
        // Typing and undoing back to the original text counts as unchanged.
        let original = EditorState::for_new();
        let mut typed = original.clone();
        typed.text = "{\"a\": 1}".into();
        assert_eq!(dismissal_action(&typed), None);
        typed.text = original.text.clone();
        assert_eq!(dismissal_action(&typed), Some(EditorAction::Cancel));
    }

    #[test]
    fn the_explicit_cancel_intent_always_cancels() {
        let mut state = EditorState::for_new();
        state.text = "changed".into();
        let (_, action) = resolve_editor(&state, EditorIntent::Cancel);
        assert_eq!(action, Some(EditorAction::Cancel));
    }
}
