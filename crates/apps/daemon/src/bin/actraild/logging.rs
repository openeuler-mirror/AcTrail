//! Logging setup for the actraild process.

use std::fmt;
use std::mem::MaybeUninit;

use tracing::{Event, Level, Subscriber};
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;

pub fn install() -> Result<(), String> {
    BracketedLevelFormat::beijing_time()
        .ok_or_else(|| "read Beijing time for daemon logging".to_string())?;
    tracing_subscriber::fmt()
        .event_format(BracketedLevelFormat)
        .with_max_level(Level::DEBUG)
        .with_writer(std::io::stderr)
        .try_init()
        .map_err(|error| format!("install daemon tracing subscriber: {error}"))
}

struct BracketedLevelFormat;

impl BracketedLevelFormat {
    fn beijing_time() -> Option<libc::tm> {
        // UTC+8 is the required log timezone, independent of the process TZ.
        const OFFSET: libc::time_t = 8 * 60 * 60;
        let now = unsafe { libc::time(std::ptr::null_mut()) };
        if now == -1 {
            return None;
        }
        let shifted = now.checked_add(OFFSET)?;
        let mut time = MaybeUninit::<libc::tm>::uninit();
        if unsafe { libc::gmtime_r(&shifted, time.as_mut_ptr()) }.is_null() {
            None
        } else {
            Some(unsafe { time.assume_init() })
        }
    }

    fn timestamp(&self, writer: &mut Writer<'_>) -> fmt::Result {
        let Some(time) = Self::beijing_time() else {
            return write!(writer, "time_unavailable");
        };
        write!(
            writer,
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}+08:00",
            time.tm_year + 1900,
            time.tm_mon + 1,
            time.tm_mday,
            time.tm_hour,
            time.tm_min,
            time.tm_sec
        )
    }
}

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
        self.timestamp(&mut writer)?;
        let level = match *event.metadata().level() {
            Level::TRACE => "trace",
            Level::DEBUG => "debug",
            Level::INFO => "info",
            Level::WARN => "warn",
            Level::ERROR => "error",
        };
        write!(writer, " [{level}] ")?;
        context
            .field_format()
            .format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}
