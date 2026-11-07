use super::*;
use uuid::Uuid;

fn exe(path: &str) -> ToolChoice {
    ToolChoice::Executable(path.into())
}

#[test]
fn nothing_set_means_the_path_search() {
    let settings = ToolSettings::default();
    assert_eq!(
        settings.effective(ToolKind::Mongosh, None),
        ToolChoice::Path
    );
    assert_eq!(
        settings.effective(ToolKind::Mongosh, Some(Uuid::new_v4())),
        ToolChoice::Path
    );
}

#[test]
fn the_global_choice_applies_to_every_connection() {
    let settings = ToolSettings::default().with_global(ToolKind::Mongosh, exe("/opt/mongosh"));
    let id = Uuid::new_v4();
    assert_eq!(
        settings.effective(ToolKind::Mongosh, Some(id)),
        exe("/opt/mongosh")
    );
    assert_eq!(
        settings.effective(ToolKind::Mongodump, Some(id)),
        ToolChoice::Path
    );
}

#[test]
fn a_connection_override_beats_the_global_choice() {
    let id = Uuid::new_v4();
    let settings = ToolSettings::default()
        .with_global(ToolKind::Mongosh, exe("/opt/global"))
        .with_override(id, ToolKind::Mongosh, exe("/opt/mine"));
    assert_eq!(
        settings.effective(ToolKind::Mongosh, Some(id)),
        exe("/opt/mine")
    );
    assert_eq!(
        settings.effective(ToolKind::Mongosh, Some(Uuid::new_v4())),
        exe("/opt/global")
    );
    assert_eq!(
        settings.effective(ToolKind::Mongosh, None),
        exe("/opt/global")
    );
}

#[test]
fn an_override_can_pick_the_path_search_over_a_global_executable() {
    let id = Uuid::new_v4();
    let settings = ToolSettings::default()
        .with_global(ToolKind::Mongosh, exe("/opt/global"))
        .with_override(id, ToolKind::Mongosh, ToolChoice::Path);
    assert_eq!(
        settings.effective(ToolKind::Mongosh, Some(id)),
        ToolChoice::Path
    );
}

#[test]
fn removing_an_override_goes_back_to_the_global_choice() {
    let id = Uuid::new_v4();
    let settings = ToolSettings::default()
        .with_global(ToolKind::Mongosh, exe("/opt/global"))
        .with_override(id, ToolKind::Mongosh, exe("/opt/mine"))
        .without_override(id, ToolKind::Mongosh);
    assert!(!settings.overrides(id, ToolKind::Mongosh));
    assert_eq!(
        settings.effective(ToolKind::Mongosh, Some(id)),
        exe("/opt/global")
    );
}

#[test]
fn removing_the_last_override_leaves_no_empty_entry() {
    let id = Uuid::new_v4();
    let settings = ToolSettings::default()
        .with_override(id, ToolKind::Mongosh, exe("/x"))
        .without_override(id, ToolKind::Mongosh);
    assert_eq!(settings, ToolSettings::default());
}

#[test]
fn changes_return_new_values_and_leave_the_original_alone() {
    let original = ToolSettings::default();
    let _changed = original.with_global(ToolKind::Mongosh, exe("/x"));
    assert_eq!(original, ToolSettings::default());
}

#[test]
fn forgetting_a_connection_drops_all_its_overrides() {
    let id = Uuid::new_v4();
    let other = Uuid::new_v4();
    let settings = ToolSettings::default()
        .with_override(id, ToolKind::Mongosh, exe("/a"))
        .with_override(id, ToolKind::Mongodump, exe("/b"))
        .with_override(other, ToolKind::Mongosh, exe("/c"));
    let after = settings.without_connection(id);
    assert!(!after.overrides(id, ToolKind::Mongosh));
    assert!(!after.overrides(id, ToolKind::Mongodump));
    assert!(after.overrides(other, ToolKind::Mongosh));
}

#[test]
fn a_duplicated_connection_starts_with_the_same_overrides() {
    let from = Uuid::new_v4();
    let to = Uuid::new_v4();
    let settings = ToolSettings::default().with_override(from, ToolKind::Mongosh, exe("/a"));
    let after = settings.with_connection_copied(from, to);
    assert_eq!(after.effective(ToolKind::Mongosh, Some(to)), exe("/a"));
    assert!(after.overrides(from, ToolKind::Mongosh));
    assert_eq!(settings.with_connection_copied(to, from), settings);
}

#[test]
fn the_settings_round_trip_through_json() {
    let id = Uuid::new_v4();
    let settings = ToolSettings::default()
        .with_global(ToolKind::Mongodump, exe("/opt/dump"))
        .with_override(id, ToolKind::Mongosh, ToolChoice::Path);
    let json = serde_json::to_string(&settings).unwrap();
    assert_eq!(
        serde_json::from_str::<ToolSettings>(&json).unwrap(),
        settings
    );
}

#[test]
fn an_empty_object_is_the_defaults() {
    assert_eq!(
        serde_json::from_str::<ToolSettings>("{}").unwrap(),
        ToolSettings::default()
    );
}

#[test]
fn the_file_holds_no_connection_details_beyond_the_id() {
    let id = Uuid::new_v4();
    let json = serde_json::to_string(&ToolSettings::default().with_override(
        id,
        ToolKind::Mongosh,
        exe("/a"),
    ))
    .unwrap();
    assert!(json.contains(&id.to_string()));
    assert!(!json.contains("uri"));
}
