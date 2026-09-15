//! Transaction coordination and consistency guards.

use std::cell::RefCell;
use std::rc::Rc;

use rusqlite::Connection;
use store_tx_contract::StorageTransactionError;
use store_tx_contract::boundary::{StorageTransaction, TransactionBoundary};

use crate::SqliteStorage;

struct SqliteWriteTransaction {
    connection: Rc<RefCell<Connection>>,
    event_payload_dictionary: Rc<RefCell<crate::records::EventPayloadDictionary>>,
    event_path_dictionary: Rc<RefCell<crate::records::PathInterner>>,
    event_record_blocks: Rc<RefCell<crate::records::EventRecordBlockWriter>>,
}

impl StorageTransaction for SqliteWriteTransaction {
    fn commit(self: Box<Self>) -> Result<(), StorageTransactionError> {
        let terminal_flush_result = self
            .event_record_blocks
            .borrow_mut()
            .flush_terminal_traces(&self.connection.borrow());
        let persist_result = terminal_flush_result.and_then(|()| {
            self.event_record_blocks
                .borrow()
                .persist_transaction_state(&self.connection.borrow())
        });
        let persist_result = persist_result.map_err(|error| {
            StorageTransactionError::new(
                "persist_event_transaction_state",
                format!("{}: {}", error.stage, error.message),
            )
        });
        let result = match persist_result {
            Ok(()) => self
                .connection
                .borrow_mut()
                .execute_batch("COMMIT")
                .map_err(|error| StorageTransactionError::new("commit", error.to_string())),
            Err(error) => Err(error),
        };
        if result.is_ok() {
            self.event_payload_dictionary
                .borrow_mut()
                .commit_transaction();
            self.event_path_dictionary.borrow_mut().commit_transaction();
            self.event_record_blocks.borrow_mut().commit_transaction();
        } else {
            let rollback_result = self.connection.borrow_mut().execute_batch("ROLLBACK");
            self.event_payload_dictionary
                .borrow_mut()
                .rollback_transaction();
            self.event_path_dictionary
                .borrow_mut()
                .rollback_transaction();
            if rollback_result.is_ok() {
                self.event_record_blocks.borrow_mut().rollback_transaction();
            } else {
                self.event_record_blocks.borrow_mut().poison_transaction();
            }
        }
        result
    }

    fn rollback(self: Box<Self>) -> Result<(), StorageTransactionError> {
        let result = self
            .connection
            .borrow_mut()
            .execute_batch("ROLLBACK")
            .map_err(|error| StorageTransactionError::new("rollback", error.to_string()));
        self.event_payload_dictionary
            .borrow_mut()
            .rollback_transaction();
        self.event_path_dictionary
            .borrow_mut()
            .rollback_transaction();
        if result.is_ok() {
            self.event_record_blocks.borrow_mut().rollback_transaction();
        } else {
            self.event_record_blocks.borrow_mut().poison_transaction();
        }
        result
    }
}

impl TransactionBoundary for SqliteStorage {
    fn begin(&mut self) -> Result<Box<dyn StorageTransaction>, StorageTransactionError> {
        self.connection()
            .borrow_mut()
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|error| StorageTransactionError::new("begin", error.to_string()))?;
        let block_begin = self
            .event_record_blocks()
            .borrow_mut()
            .begin_transaction(&self.connection().borrow());
        if let Err(error) = block_begin {
            let rollback_result = self.connection().borrow_mut().execute_batch("ROLLBACK");
            if rollback_result.is_err() {
                self.event_record_blocks().borrow_mut().poison_transaction();
            }
            return Err(StorageTransactionError::new(error.stage, error.message));
        }
        self.event_payload_dictionary()
            .borrow_mut()
            .begin_transaction();
        self.event_path_dictionary()
            .borrow_mut()
            .begin_transaction();
        Ok(Box::new(SqliteWriteTransaction {
            connection: self.connection().clone(),
            event_payload_dictionary: self.event_payload_dictionary().clone(),
            event_path_dictionary: self.event_path_dictionary().clone(),
            event_record_blocks: self.event_record_blocks().clone(),
        }))
    }
}
