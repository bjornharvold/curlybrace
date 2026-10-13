//! Data compare: key ordering, normalization, fingerprints, diff, scanning and the merge-join
//! engine. Pure core logic with no UI dependency; the runtime drives it (see `runtime::compare`).

pub mod apply;
pub mod diff;
pub mod engine;
pub mod fingerprint;
pub mod key;
pub mod keyset;
pub mod normalize;
pub(crate) mod number;
pub mod pairing;
pub mod protocol;
pub mod results;
pub mod rules;
pub mod scan;
