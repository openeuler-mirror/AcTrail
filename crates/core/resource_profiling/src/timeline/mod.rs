//! Windowed, downsampled resource tracks for the flame graph `res` area.

mod resource_timeline;
mod resource_timeline_reader;
mod timeline_builder;
pub use resource_timeline::{
    Gap, HostPoint, HostTrack, ResourceTimeline, TrackReason, WorkloadPoint, WorkloadSource,
    WorkloadSummary, WorkloadTrack,
};
pub use resource_timeline_reader::{
    DEFAULT_MAX_POINTS, DEFAULT_ROW_LIMIT, MAX_POINTS_LIMIT, ResourceTimelineReader,
    ResourceTimelineRequest,
};
