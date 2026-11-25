//! The file under a buffered writer: what an export renders into.

use std::fs::File;
use std::io::{self, BufWriter};
use std::path::Path;

/// Buffer between the renderer and the file.
const BUFFER: usize = 64 * 1024;

/// The file under the buffer. It can be told to fail after some bytes, which only tests do.
pub(in crate::runtime) struct ExportSink {
    file: File,
    fail_after: Option<u64>,
    written: u64,
}

impl ExportSink {
    pub(in crate::runtime) fn new(file: File, path: &Path) -> Self {
        #[cfg(test)]
        let fail_after = super::test_hooks::fail_after(path);
        #[cfg(not(test))]
        let fail_after = {
            let _ = path;
            None
        };
        Self {
            file,
            fail_after,
            written: 0,
        }
    }
}

impl io::Write for ExportSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.fail_after.is_some_and(|limit| self.written >= limit) {
            return Err(io::Error::other("injected write failure"));
        }
        let n = self.file.write(buf)?;
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

pub(in crate::runtime) type Out = BufWriter<ExportSink>;

/// The file under the writer's buffer.
pub(in crate::runtime) fn buffered(sink: ExportSink) -> Out {
    BufWriter::with_capacity(BUFFER, sink)
}

/// Flushes `out` and hands the file back.
pub(in crate::runtime) fn into_file(out: Out) -> io::Result<File> {
    out.into_inner()
        .map(|sink| sink.file)
        .map_err(|e| e.into_error())
}
