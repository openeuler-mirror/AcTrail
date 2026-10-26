use crate::resource_state::ResourceState;
use std::cell::RefCell;
use std::rc::Rc;
use storage_core::{StorageError, StorageTransaction};

pub(super) struct NoOpTransaction {
    resources: Rc<RefCell<ResourceState>>,
    finished: bool,
}

impl NoOpTransaction {
    pub(super) fn begin(resources: Rc<RefCell<ResourceState>>) -> Result<Self, StorageError> {
        resources.borrow_mut().begin()?;
        Ok(Self {
            resources,
            finished: false,
        })
    }
}

impl Drop for NoOpTransaction {
    fn drop(&mut self) {
        if !self.finished {
            self.resources.borrow_mut().finish(false);
        }
    }
}

impl StorageTransaction for NoOpTransaction {
    fn commit(mut self: Box<Self>) -> Result<(), StorageError> {
        self.resources.borrow_mut().finish(true);
        self.finished = true;
        Ok(())
    }

    fn rollback(mut self: Box<Self>) -> Result<(), StorageError> {
        self.resources.borrow_mut().finish(false);
        self.finished = true;
        Ok(())
    }
}
