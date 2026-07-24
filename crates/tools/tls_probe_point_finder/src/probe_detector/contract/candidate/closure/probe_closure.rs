use crate::plan::{PayloadDirection, ProbePoint};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProbeClosure {
    pub(crate) outbound: usize,
    pub(crate) inbound: usize,
}

impl ProbeClosure {
    pub(crate) fn from_points(points: &[ProbePoint]) -> Option<Self> {
        let outbound = points
            .iter()
            .filter(|point| point.direction == PayloadDirection::Outbound)
            .count();
        let inbound = points
            .iter()
            .filter(|point| point.direction == PayloadDirection::Inbound)
            .count();
        (outbound > 0 && inbound > 0).then_some(Self { outbound, inbound })
    }
}
