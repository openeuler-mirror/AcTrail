use std::io;
use std::sync::Arc;

pub(super) trait Decoder: Send {
    fn append(&mut self, bytes: &[u8]) -> io::Result<()>;
    fn finish(&mut self) -> io::Result<()>;
    fn snapshot(&self) -> Arc<Vec<u8>>;
}
