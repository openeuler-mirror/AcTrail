use crate::{GuestBootId, ProcessMarker};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OomVictimAttribution {
    Unknown,
    Monitored,
    Unmonitored,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OomVictimObservation {
    pub guest_boot_id: GuestBootId,
    pub detected_at_ms: u64,
    pub victim_pid: u32,
    pub victim_comm: [u8; 16],
    pub attribution: OomVictimAttribution,
    pub monitored_root: Option<ProcessMarker>,
}

impl OomVictimObservation {
    pub fn validate(self) -> Result<Self, &'static str> {
        match (self.attribution, self.monitored_root) {
            (OomVictimAttribution::Monitored, Some(_)) => Ok(self),
            (OomVictimAttribution::Unknown | OomVictimAttribution::Unmonitored, None) => Ok(self),
            _ => Err("OOM victim attribution and monitored root disagree"),
        }
    }
}
