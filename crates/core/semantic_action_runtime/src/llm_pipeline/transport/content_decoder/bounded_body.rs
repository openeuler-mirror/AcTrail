use std::io::{self, Write};
use std::sync::Arc;

pub(super) struct BoundedBody {
    bytes: Arc<Vec<u8>>,
    max_bytes: usize,
}

impl BoundedBody {
    pub(super) fn new(max_bytes: usize) -> Self {
        Self {
            bytes: Arc::new(Vec::new()),
            max_bytes,
        }
    }

    pub(super) fn snapshot(&self) -> Arc<Vec<u8>> {
        Arc::clone(&self.bytes)
    }

    pub(super) fn clear(&mut self) {
        if let Some(bytes) = Arc::get_mut(&mut self.bytes) {
            bytes.clear();
        } else {
            self.bytes = Arc::new(Vec::new());
        }
    }
}

impl Write for BoundedBody {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self
            .bytes
            .len()
            .checked_add(bytes.len())
            .is_none_or(|size| size > self.max_bytes)
        {
            return Err(io::Error::new(
                io::ErrorKind::OutOfMemory,
                "decoded HTTP body exceeds assembly limit",
            ));
        }
        Arc::make_mut(&mut self.bytes).extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
