use std::collections::HashSet;
use std::io;
use std::sync::Mutex;

#[derive(Debug)]
pub(super) struct SessionRegistry {
    state: Mutex<RegistryState>,
    capacity: usize,
}

#[derive(Debug)]
struct RegistryState {
    next_id: u32,
    active: HashSet<u32>,
}

impl SessionRegistry {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            state: Mutex::new(RegistryState {
                next_id: 1,
                active: HashSet::with_capacity(capacity),
            }),
            capacity,
        }
    }

    pub(super) fn allocate(&self) -> io::Result<u32> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("SB session registry lock poisoned"))?;
        if state.active.len() >= self.capacity {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionRefused,
                "SB connection capacity reached",
            ));
        }
        for _ in 0..=self.capacity {
            let candidate = state.next_id.max(1);
            state.next_id = candidate.wrapping_add(1).max(1);
            if state.active.insert(candidate) {
                return Ok(candidate);
            }
        }
        Err(io::Error::other("no free SB numeric ID"))
    }

    pub(super) fn release(&self, id: u32) {
        if let Ok(mut state) = self.state.lock() {
            state.active.remove(&id);
        }
    }

    pub(super) fn active_count(&self) -> usize {
        self.state
            .lock()
            .map(|state| state.active.len())
            .unwrap_or(self.capacity)
    }
}
