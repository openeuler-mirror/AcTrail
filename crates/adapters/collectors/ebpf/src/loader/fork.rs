//! sched_process_fork tracepoint format configuration.

use libbpf_rs::{MapCore, MapFlags, Object};

use super::{AttachPlan, LoaderError, object, tracepoint};

const FORK_PROGRAM: &str = "handle_sched_process_fork";
const FORK_CHILD_PID_OFFSET_MAP: &str = "fork_child_pid_offset";
const FORK_CHILD_PID_OFFSET_KEY: u32 = 0;
const FORK_PARENT_PID_OFFSET_KEY: u32 = 1;

pub(super) fn configure_child_pid_offset_map(
    object: &Object,
    attach_plan: &AttachPlan,
) -> Result<(), LoaderError> {
    if !attach_plan.should_load_program(FORK_PROGRAM)? {
        return Ok(());
    }

    let map = object::map_handle(object, FORK_CHILD_PID_OFFSET_MAP, "fork_child_pid_offset")?;
    let offsets = read_pid_offsets()?;
    map.update(
        &FORK_CHILD_PID_OFFSET_KEY.to_ne_bytes(),
        &offsets.child_pid.to_ne_bytes(),
        MapFlags::ANY,
    )
    .map_err(|error| LoaderError::new("fork_child_pid_offset", error.to_string()))?;
    map.update(
        &FORK_PARENT_PID_OFFSET_KEY.to_ne_bytes(),
        &offsets.parent_pid.to_ne_bytes(),
        MapFlags::ANY,
    )
    .map_err(|error| LoaderError::new("fork_child_pid_offset", error.to_string()))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ForkPidOffsets {
    parent_pid: u32,
    child_pid: u32,
}

fn read_pid_offsets() -> Result<ForkPidOffsets, LoaderError> {
    let roots = tracepoint::tracefs_roots()?;
    let mut errors = Vec::new();
    for root in roots {
        let path = root
            .join("events")
            .join("sched")
            .join("sched_process_fork")
            .join("format");
        match std::fs::read_to_string(&path) {
            Ok(content) => match parse_pid_offsets(&content) {
                Some(offsets) => return Ok(offsets),
                None => errors.push(format!(
                    "{}: parent_pid or child_pid offset is missing or invalid",
                    path.display()
                )),
            },
            Err(error) => errors.push(format!("{}: {error}", path.display())),
        }
    }
    Err(LoaderError::new(
        "fork_child_pid_offset",
        format!(
            "sched_process_fork pid offsets are unavailable: {}",
            errors.join("; ")
        ),
    ))
}

fn parse_pid_offsets(content: &str) -> Option<ForkPidOffsets> {
    let parent_pid = parse_field_offset(content, "parent_pid")?;
    let child_pid = parse_field_offset(content, "child_pid")?;
    Some(ForkPidOffsets {
        parent_pid,
        child_pid,
    })
}

fn parse_field_offset(content: &str, field_name: &str) -> Option<u32> {
    for line in content.lines().map(str::trim) {
        let mut parts = line.split(';').map(str::trim);
        let Some(field) = parts.next() else {
            continue;
        };
        if !is_named_field(field, field_name) {
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

fn is_named_field(field: &str, field_name: &str) -> bool {
    field
        .strip_prefix("field:")
        .and_then(|field| field.split_whitespace().last())
        == Some(field_name)
}

#[cfg(test)]
mod tests {
    use super::{ForkPidOffsets, parse_pid_offsets};

    #[test]
    fn parses_dynamic_sched_process_fork_pid_offsets() {
        let format = r#"
name: sched_process_fork
ID: 299
format:
	field:unsigned short common_type;	offset:0;	size:2;	signed:0;
	field:unsigned char common_flags;	offset:2;	size:1;	signed:0;
	field:unsigned char common_preempt_count;	offset:3;	size:1;	signed:0;
	field:int common_pid;	offset:4;	size:4;	signed:1;

	field:__data_loc char[] parent_comm;	offset:8;	size:4;	signed:0;
	field:pid_t parent_pid;	offset:12;	size:4;	signed:1;
	field:__data_loc char[] child_comm;	offset:16;	size:4;	signed:0;
	field:pid_t child_pid;	offset:20;	size:4;	signed:1;
"#;

        assert_eq!(
            parse_pid_offsets(format),
            Some(ForkPidOffsets {
                parent_pid: 12,
                child_pid: 20,
            })
        );
    }

    #[test]
    fn rejects_missing_sched_process_fork_pid_offset() {
        let format = r#"
	field:pid_t parent_pid;	offset:12;	size:4;	signed:1;
"#;

        assert_eq!(parse_pid_offsets(format), None);
    }
}
