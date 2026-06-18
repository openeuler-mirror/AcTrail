//! sched_process_fork tracepoint format configuration.

use libbpf_rs::{MapCore, MapFlags, Object};

use super::{AttachPlan, LoaderError, object, tracepoint};

const FORK_PROGRAM: &str = "handle_sched_process_fork";
const FORK_CHILD_PID_OFFSET_MAP: &str = "fork_child_pid_offset";
const FORK_CHILD_PID_OFFSET_KEY: u32 = 0;

pub(super) fn configure_child_pid_offset_map(
    object: &Object,
    attach_plan: &AttachPlan,
) -> Result<(), LoaderError> {
    if !attach_plan.should_load_program(FORK_PROGRAM)? {
        return Ok(());
    }

    let map = object::map_handle(object, FORK_CHILD_PID_OFFSET_MAP, "fork_child_pid_offset")?;
    let offset = read_child_pid_offset()?;
    map.update(
        &FORK_CHILD_PID_OFFSET_KEY.to_ne_bytes(),
        &offset.to_ne_bytes(),
        MapFlags::ANY,
    )
    .map_err(|error| LoaderError::new("fork_child_pid_offset", error.to_string()))
}

fn read_child_pid_offset() -> Result<u32, LoaderError> {
    let roots = tracepoint::tracefs_roots()?;
    let mut errors = Vec::new();
    for root in roots {
        let path = root
            .join("events")
            .join("sched")
            .join("sched_process_fork")
            .join("format");
        match std::fs::read_to_string(&path) {
            Ok(content) => match parse_child_pid_offset(&content) {
                Some(offset) => return Ok(offset),
                None => errors.push(format!(
                    "{}: child_pid offset is missing or invalid",
                    path.display()
                )),
            },
            Err(error) => errors.push(format!("{}: {error}", path.display())),
        }
    }
    Err(LoaderError::new(
        "fork_child_pid_offset",
        format!(
            "sched_process_fork child_pid offset is unavailable: {}",
            errors.join("; ")
        ),
    ))
}

fn parse_child_pid_offset(content: &str) -> Option<u32> {
    for line in content.lines().map(str::trim) {
        let mut parts = line.split(';').map(str::trim);
        let Some(field) = parts.next() else {
            continue;
        };
        if !is_child_pid_field(field) {
            continue;
        }
        for part in parts {
            let Some(value) = part.strip_prefix("offset:") else {
                continue;
            };
            let offset = value.trim().parse::<u32>().ok()?;
            if offset == 0 {
                return None;
            }
            return Some(offset);
        }
        return None;
    }
    None
}

fn is_child_pid_field(field: &str) -> bool {
    field
        .strip_prefix("field:")
        .and_then(|field| field.split_whitespace().last())
        == Some("child_pid")
}
