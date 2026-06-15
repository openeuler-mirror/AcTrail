#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportError {
    pub code: String,
    pub message: String,
    queue_capacity: Option<u32>,
}

impl ExportError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            queue_capacity: None,
        }
    }

    pub fn with_queue_capacity(mut self, queue_capacity: u32) -> Self {
        self.queue_capacity = Some(queue_capacity);
        self
    }

    pub const fn queue_capacity(&self) -> Option<u32> {
        self.queue_capacity
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExportPublishResult {
    dropped: Option<ExportDeliveryDrop>,
}

impl ExportPublishResult {
    pub const fn delivered() -> Self {
        Self { dropped: None }
    }

    pub const fn dropped(drop: ExportDeliveryDrop) -> Self {
        Self {
            dropped: Some(drop),
        }
    }

    pub const fn dropped_records(self) -> u64 {
        match self.dropped {
            Some(drop) => drop.dropped_records,
            None => 0,
        }
    }

    pub const fn dropped_outcome(self) -> Option<ExportDeliveryDrop> {
        self.dropped
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExportDeliveryDrop {
    reason: ExportDropReason,
    dropped_records: u64,
    queue_capacity: Option<u32>,
}

impl ExportDeliveryDrop {
    pub const fn queue_full(dropped_records: u64, queue_capacity: u32) -> Self {
        Self {
            reason: ExportDropReason::QueueFull,
            dropped_records,
            queue_capacity: Some(queue_capacity),
        }
    }

    pub const fn reason(self) -> ExportDropReason {
        self.reason
    }

    pub const fn dropped_records(self) -> u64 {
        self.dropped_records
    }

    pub const fn queue_capacity(self) -> Option<u32> {
        self.queue_capacity
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportDropReason {
    QueueFull,
}

impl ExportDropReason {
    pub const fn code(self) -> &'static str {
        match self {
            Self::QueueFull => "queue_full",
        }
    }
}
