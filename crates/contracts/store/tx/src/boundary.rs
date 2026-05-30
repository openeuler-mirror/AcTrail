//! Transaction-boundary contracts for storage implementations.

use crate::StorageTransactionError;

pub trait StorageTransaction {
    fn commit(self: Box<Self>) -> Result<(), StorageTransactionError>;
    fn rollback(self: Box<Self>) -> Result<(), StorageTransactionError>;
}

pub trait TransactionBoundary {
    fn begin(&mut self) -> Result<Box<dyn StorageTransaction>, StorageTransactionError>;
}
