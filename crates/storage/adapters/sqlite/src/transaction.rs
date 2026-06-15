//! Transaction coordination and consistency guards.

use std::cell::RefCell;
use std::rc::Rc;

use rusqlite::Connection;
use store_tx_contract::StorageTransactionError;
use store_tx_contract::boundary::{StorageTransaction, TransactionBoundary};

use crate::SqliteStorage;

struct SqliteWriteTransaction {
    connection: Rc<RefCell<Connection>>,
}

impl StorageTransaction for SqliteWriteTransaction {
    fn commit(self: Box<Self>) -> Result<(), StorageTransactionError> {
        self.connection
            .borrow_mut()
            .execute_batch("COMMIT")
            .map_err(|error| StorageTransactionError::new("commit", error.to_string()))
    }

    fn rollback(self: Box<Self>) -> Result<(), StorageTransactionError> {
        self.connection
            .borrow_mut()
            .execute_batch("ROLLBACK")
            .map_err(|error| StorageTransactionError::new("rollback", error.to_string()))
    }
}

impl TransactionBoundary for SqliteStorage {
    fn begin(&mut self) -> Result<Box<dyn StorageTransaction>, StorageTransactionError> {
        self.connection()
            .borrow_mut()
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|error| StorageTransactionError::new("begin", error.to_string()))?;
        Ok(Box::new(SqliteWriteTransaction {
            connection: self.connection().clone(),
        }))
    }
}
