//! The administration entries of the sidebar's row menus: rename, duplicate, create a view, drop.
//! On a read-only connection they are listed but off, with the reason on hover; they only open a
//! dialog, which asks for the details (and the typed name of a drop) before anything is sent.
use super::sidebar_copy::TreeCtx;
use crate::I18n;
use crate::viewmodel::connection_tag::READ_ONLY_REASON_KEY;
use crate::viewmodel::{AdminKind, AdminRequest, SidebarAction};
use eframe::egui;

/// The entries and the label key of each, for a database row (`collection == None`) or a
/// collection row.
fn entries(collection: Option<&str>) -> &'static [(AdminKind, &'static str)] {
    match collection {
        Some(_) => &[
            (AdminKind::Rename, "sidebar.admin.rename"),
            (AdminKind::Duplicate, "sidebar.admin.duplicate"),
            (AdminKind::CreateView, "sidebar.admin.create_view_from"),
            (AdminKind::DropCollection, "sidebar.admin.drop_collection"),
        ],
        None => &[
            (AdminKind::CreateView, "sidebar.admin.create_view"),
            (AdminKind::DropDatabase, "sidebar.admin.drop_database"),
        ],
    }
}

/// Adds the entries to the open menu `ui`; returns the one clicked this frame. A row of a
/// placeholder session (no tree context) has none.
pub fn admin_entries(
    ui: &mut egui::Ui,
    ctx: Option<&TreeCtx>,
    db: &str,
    collection: Option<&str>,
    i18n: &I18n,
) -> Option<SidebarAction> {
    let ctx = ctx?;
    let read_only = ctx.state.session_read_only(ctx.session);
    ui.separator();
    let mut chosen = None;
    for (kind, label_key) in entries(collection) {
        let entry = ui.add_enabled(!read_only, egui::Button::new(i18n.t(label_key)));
        if read_only {
            entry.on_disabled_hover_text(i18n.t(READ_ONLY_REASON_KEY));
        } else if entry.clicked() {
            chosen = Some(SidebarAction::Admin(AdminRequest {
                kind: *kind,
                db: db.to_owned(),
                collection: collection.map(str::to_owned),
            }));
            ui.close();
        }
    }
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_collection_row_offers_four_entries_and_a_database_row_two() {
        let kinds = |c| entries(c).iter().map(|(k, _)| *k).collect::<Vec<_>>();
        assert_eq!(
            kinds(Some("c")),
            [
                AdminKind::Rename,
                AdminKind::Duplicate,
                AdminKind::CreateView,
                AdminKind::DropCollection
            ]
        );
        assert_eq!(
            kinds(None),
            [AdminKind::CreateView, AdminKind::DropDatabase]
        );
    }
}
