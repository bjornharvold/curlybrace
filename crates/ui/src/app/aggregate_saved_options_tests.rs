//! Save and open of saved pipelines with the draft's options, through the reducer, with a real
//! `AppState` and a `pipelines.json` in a tempdir.
use super::tests::{act, drafted, only_entry, saved_as, session_in};
use crate::app::AggregateAction as A;
use crate::app::OptionsAction as O;
use crate::viewmodel::aggregate_options::{OptionsEdit, OptionsForm, Tri};
use curlybrace_core::pipeline::{PipelineOptions, ReadPreferenceMode};
use uuid::Uuid;

fn with_options(s: crate::app::state::AppState) -> crate::app::state::AppState {
    let s = act(
        s,
        A::Options(O::Edit(OptionsEdit::ReadPreference(Some(
            ReadPreferenceMode::SecondaryPreferred,
        )))),
    )
    .0;
    act(s, A::Options(O::Edit(OptionsEdit::AllowDiskUse(Tri::On)))).0
}

#[test]
fn save_stores_the_draft_options() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let s = saved_as(with_options(drafted(session_in(&dir, conn))), "Totals");
    let entry = only_entry(&s, conn);
    assert_eq!(entry.options, s.aggregate.options);
    assert_eq!(entry.options.set_count(), 2);
    assert!(!s.aggregate.is_modified());
}

#[test]
fn save_is_refused_while_the_form_is_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let s = with_options(drafted(session_in(&dir, conn)));
    let s = act(s, A::Options(O::Edit(OptionsEdit::MaxTime("0".into())))).0;
    let s = saved_as(s, "Totals");
    assert!(s.aggregate.saved.dialog.is_some(), "the dialog stays");
    assert_eq!(
        s.aggregate.saved.error,
        Some("aggregate.saved.error.invalid_options")
    );
    assert!(s.pipelines.list(conn).is_empty(), "nothing was saved");
    assert!(
        !dir.path().join("pipelines.json").exists(),
        "file untouched"
    );
}

#[test]
fn resaving_an_opened_pipeline_replaces_its_options() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let s = saved_as(with_options(drafted(session_in(&dir, conn))), "Totals");
    let (id, name) = s.aggregate.saved.opened.clone().expect("opened");
    // Reset the options, then save over the same name.
    let s = act(s, A::Options(O::Reset)).0;
    assert!(s.aggregate.is_modified());
    let s = act(s, A::SaveClicked).0;
    let s = act(s, A::SaveConfirmed).0;
    let s = act(s, A::SaveConfirmed).0;
    let entry = only_entry(&s, conn);
    assert_eq!((entry.id, entry.name.as_str()), (id, name.as_str()));
    assert!(entry.options.is_empty());
    assert!(!s.aggregate.is_modified());
}

#[test]
fn opening_restores_the_options_through_the_reducer() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let s = saved_as(with_options(drafted(session_in(&dir, conn))), "Totals");
    let entry = only_entry(&s, conn);
    let s = act(s, A::Options(O::Reset)).0;
    let s = act(s, A::OpenMenuPicked(entry.id)).0;
    // Modified (options reset), so it asks first.
    let s = act(s, A::OpenConfirmed).0;
    assert_eq!(s.aggregate.options, entry.options);
    assert_eq!(
        s.aggregate.options_ui.form,
        OptionsForm::from_options(&entry.options)
    );
    assert_ne!(s.aggregate.options, PipelineOptions::default());
}
