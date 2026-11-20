//! A write refused as too big is fitted again.
use super::*;

#[test]
fn a_write_refused_for_size_is_fitted_again_and_written() {
    let mut rig = rig(true);
    let id = rig.id;
    let big: Vec<crate::app::workspace_file::SavedTab> = (0..40)
        .map(|n| crate::app::workspace_file::SavedTab {
            connection: id,
            db: "app".into(),
            collection: format!("c{n}"),
            sub_tab: crate::app::workspace_file::SavedSubTab::Documents,
            view_mode: crate::app::workspace_file::SavedViewMode::Table,
            find: Some(crate::app::workspace_file::SavedFind {
                filter: "x".repeat(250 * 1024),
                ..Default::default()
            }),
            draft: None,
            text_dropped: false,
            options_dropped: false,
        })
        .collect();
    let oversized = WorkspaceFile {
        tabs: big,
        ..WorkspaceFile::default()
    };
    rig.app.state_mut().workspace_written(oversized);
    let sync = &rig.app.state().restore.sync;
    assert_eq!(sync.writes, 1, "it was written after all");
    assert!(
        sync.failed.is_none() && sync.streak.is_none(),
        "and nothing is parked"
    );
    let size = std::fs::metadata(rig.path()).unwrap().len();
    assert!(
        size <= crate::app::workspace_file::MAX_FILE_BYTES as u64,
        "{size}"
    );
    assert!(rig.app.state().banner.is_none());
}
