//! Data compare: key ordering, normalization, fingerprints, diff, scanning and the merge-join
//! engine. Pure core logic with no UI dependency; the runtime drives it (see `runtime::compare`).

pub mod diff;
pub mod fingerprint;
pub mod key;
pub mod keyset;
pub mod normalize;
pub(crate) mod number;
pub mod rules;
