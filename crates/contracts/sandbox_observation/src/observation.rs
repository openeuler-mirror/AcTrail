use crate::{GuestResourceSnapshot, ProcessIoCounters};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Observation {
    ProcessIo(ProcessIoCounters),
    GuestResource(GuestResourceSnapshot),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationBatch {
    pub sequence: u64,
    pub observations: Vec<Observation>,
}

impl ObservationBatch {
    pub fn new(sequence: u64, observations: Vec<Observation>) -> Self {
        Self {
            sequence,
            observations,
        }
    }
}
