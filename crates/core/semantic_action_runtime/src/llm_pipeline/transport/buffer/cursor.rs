//! Amortized O(1) prefix release for bounded transport assembly.

use std::ops::Deref;

#[derive(Default)]
pub(in crate::llm_pipeline) struct CursorBuffer {
    storage: Vec<u8>,
    head: usize,
}

impl CursorBuffer {
    pub(in crate::llm_pipeline) fn from_vec(storage: Vec<u8>) -> Self {
        Self { storage, head: 0 }
    }

    pub(in crate::llm_pipeline) fn extend_from_slice(&mut self, bytes: &[u8]) {
        self.storage.extend_from_slice(bytes);
    }

    pub(in crate::llm_pipeline) fn release(&mut self, consumed: usize) -> bool {
        if consumed > self.len() {
            return false;
        }
        self.head += consumed;
        if self.head == self.storage.len() {
            self.storage.clear();
            self.head = 0;
            return true;
        }
        let remaining = self.storage.len() - self.head;
        if self.head >= remaining {
            self.storage.copy_within(self.head.., 0);
            self.storage.truncate(remaining);
            self.head = 0;
        }
        true
    }

    pub(in crate::llm_pipeline) fn take_remaining(&mut self) -> Vec<u8> {
        if self.head == 0 {
            return std::mem::take(&mut self.storage);
        }
        let remaining = self.to_vec();
        self.storage.clear();
        self.head = 0;
        remaining
    }
}

impl Deref for CursorBuffer {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.storage[self.head..]
    }
}
