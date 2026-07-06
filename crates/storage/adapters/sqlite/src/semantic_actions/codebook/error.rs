use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CodebookError {
    stage: &'static str,
    detail: String,
}

impl CodebookError {
    pub(super) fn new(stage: &'static str, detail: impl Into<String>) -> Self {
        Self {
            stage,
            detail: detail.into(),
        }
    }

    pub(super) fn unknown(stage: &'static str, value: impl fmt::Display) -> Self {
        Self::new(
            stage,
            format!("unknown semantic action storage code {value}"),
        )
    }
}

impl fmt::Display for CodebookError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.stage, self.detail)
    }
}

impl std::error::Error for CodebookError {}

pub(super) fn validate_unique(
    stage: &'static str,
    entries: &[(&'static str, i16)],
) -> Result<(), CodebookError> {
    for (left_index, (left_name, left_code)) in entries.iter().enumerate() {
        for (right_name, right_code) in entries.iter().skip(left_index + 1) {
            if left_name == right_name {
                return Err(CodebookError::new(
                    stage,
                    format!("duplicate semantic action storage name {left_name}"),
                ));
            }
            if left_code == right_code {
                return Err(CodebookError::new(
                    stage,
                    format!(
                        "duplicate semantic action storage code {left_code} for {left_name} and {right_name}"
                    ),
                ));
            }
        }
    }
    Ok(())
}
