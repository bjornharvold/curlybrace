//! Whether a bulk flow may be opened now, and if not, why. `Open` and the documents toolbar both
//! read it from here, so the buttons are enabled exactly when opening would start a flow.
use super::state::AppState;
use crate::viewmodel::BulkEntry;

impl AppState {
    /// Read-only first, then a projected page, then a page found with a collation (a bulk write
    /// matches by the filter alone and would not use it), then a flow or a write already in the
    /// way.
    pub fn bulk_entry(&self) -> BulkEntry {
        if self.active_is_read_only() {
            BulkEntry::ReadOnly
        } else if self.page_is_projected() {
            BulkEntry::Projected
        } else if self.page_query.options.collation.is_some() {
            BulkEntry::Collated
        } else if self.bulk.is_some() || self.pending.is_some() || self.write_warning.is_some() {
            BulkEntry::Busy
        } else {
            BulkEntry::Available
        }
    }
}
