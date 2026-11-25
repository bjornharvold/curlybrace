//! Replacing the header of a compare export in place once the true counts are known. The file
//! itself (path rules, owner-only creation, removal) is `runtime::export_file`.

use std::fs::File;
use std::io::{self, Seek, SeekFrom, Write};

/// Overwrites the first `expected_len` bytes of `file` with `header`, which must have exactly
/// that length: a small in-place write, nothing is moved. Data is synced before (so the body is
/// on disk before the file claims to be complete) and after (before the caller keeps the file).
pub(super) fn overwrite_header(
    file: &mut File,
    expected_len: u64,
    header: &[u8],
) -> io::Result<()> {
    if header.len() as u64 != expected_len {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    file.sync_all()?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(header)?;
    file.flush()?;
    file.sync_all()
}
