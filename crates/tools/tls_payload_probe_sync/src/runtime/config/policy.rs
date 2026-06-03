#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RedactionMode {
    Redact,
    None,
}

impl RedactionMode {
    pub(super) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "redact" => Ok(Self::Redact),
            "none" => Ok(Self::None),
            _ => Err(format!("unknown redaction mode: {value}")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct EventFilter {
    target: bool,
    payload: bool,
    decision: bool,
}

impl EventFilter {
    pub(super) fn parse(value: Option<&str>) -> Result<Self, String> {
        let Some(value) = value else {
            return Ok(Self {
                target: true,
                payload: true,
                decision: true,
            });
        };
        let mut filter = Self {
            target: false,
            payload: false,
            decision: false,
        };
        if value.is_empty() {
            return Ok(filter);
        }
        for item in value.split(',') {
            match item {
                "target" => filter.target = true,
                "payload" => filter.payload = true,
                "decision" => filter.decision = true,
                _ => return Err(format!("unknown runtime event group: {item}")),
            }
        }
        Ok(filter)
    }

    pub(super) fn target(&self) -> bool {
        self.target
    }

    pub(super) fn payload(&self) -> bool {
        self.payload
    }

    pub(super) fn decision(&self) -> bool {
        self.decision
    }
}
