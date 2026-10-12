//! Data compare: key ordering, normalization, fingerprints, diff, scanning and the merge-join
//! engine. Pure core logic with no UI dependency; the runtime drives it (see `runtime::compare`).

pub mod key;
pub(crate) mod number;
