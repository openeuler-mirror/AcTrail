use crate::{
    GuestPressureSnapshot, GuestResourceSnapshot, OomVictimObservation, ProcessIoCounters,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Observation {
    ProcessIo(ProcessIoCounters),
    GuestResource(GuestResourceSnapshot),
    OomVictim(OomVictimObservation),
    GuestPressure(GuestPressureSnapshot),
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
