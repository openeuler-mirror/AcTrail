//! Unified storage transaction boundary.

use crate::StorageError;

pub trait StorageTransaction {
    fn commit(self: Box<Self>) -> Result<(), StorageError>;
    fn rollback(self: Box<Self>) -> Result<(), StorageError>;
}
