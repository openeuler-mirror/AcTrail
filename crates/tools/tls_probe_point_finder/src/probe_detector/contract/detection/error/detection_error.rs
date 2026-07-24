use std::error::Error;
use std::fmt::{Display, Formatter};

use crate::probe_detector::contract::identity::DetectorPath;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DetectionError {
    pub(crate) detector_path: DetectorPath,
    message: String,
}

impl DetectionError {
    pub(crate) fn new(detector_path: DetectorPath, message: impl Into<String>) -> Self {
        Self {
            detector_path,
            message: message.into(),
        }
    }
}

impl Display for DetectionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}: {}",
            self.detector_path.display(),
            self.message
        )
    }
}

impl Error for DetectionError {}
