//! Logging setup for the actraild process.

use std::fmt;

use tracing::{Event, Level, Subscriber};
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;

pub fn install() -> Result<(), String> {
    tracing_subscriber::fmt()
        .event_format(BracketedLevelFormat)
        .with_max_level(Level::DEBUG)
        .with_writer(std::io::stderr)
        .try_init()
        .map_err(|error| format!("install daemon tracing subscriber: {error}"))
}

struct BracketedLevelFormat;

impl<S, N> FormatEvent<S, N> for BracketedLevelFormat
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    fn format_event(
        &self,
        context: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        write!(
            writer,
            "[{}] ",
            event.metadata().level().as_str().to_ascii_lowercase()
        )?;
        context
            .field_format()
            .format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}
