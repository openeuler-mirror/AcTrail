use super::DetectorId;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct DetectorPath(Vec<DetectorId>);

impl DetectorPath {
    pub(crate) fn root(id: DetectorId) -> Self {
        Self(vec![id])
    }

    pub(crate) fn child(&self, id: DetectorId) -> Self {
        let mut segments = self.0.clone();
        segments.push(id);
        Self(segments)
    }

    pub(crate) fn display(&self) -> String {
        self.0
            .iter()
            .map(DetectorId::as_str)
            .collect::<Vec<_>>()
            .join("/")
    }
}
