use crate::{
    GuestPressureSnapshot, GuestResourceSnapshot, OomVictimObservation, ProcessIoCounters,
    WorkloadCgroupResourceSnapshot,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum Observation {
    ProcessIo(ProcessIoCounters),
    GuestResource(GuestResourceSnapshot),
    OomVictim(OomVictimObservation),
    GuestPressure(GuestPressureSnapshot),
    WorkloadCgroup(WorkloadCgroupResourceSnapshot),
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
