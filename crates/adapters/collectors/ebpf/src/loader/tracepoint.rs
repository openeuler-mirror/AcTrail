//! Tracepoint attach strategy for loaded eBPF programs.

use std::io;
use std::os::fd::RawFd;
use std::path::PathBuf;

use libbpf_rs::{Link, PerfEventOpts, ProgramMut, libbpf_sys};

use crate::loader::LoaderError;
use crate::loader::environment;

const TRACEPOINT_PREFIX: &str = "tracepoint/";
const TP_PREFIX: &str = "tp/";

pub(super) fn attach_program(
    program: &ProgramMut<'_>,
    program_name: &str,
    allow_missing_tracepoint: bool,
) -> Result<Option<Link>, LoaderError> {
    let Some(target) = tracepoint_target(program, program_name)? else {
        return program.attach().map(Some).map_err(|error| {
            LoaderError::new(
                "attach_program",
                format!(
                    "{}; program={}; {}",
                    error,
                    program_name,
                    environment::attach_environment_description()
                ),
            )
        });
    };
    attach_tracepoint(program, program_name, &target, allow_missing_tracepoint)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TracepointTarget {
    category: String,
    name: String,
}

impl TracepointTarget {
    fn display(&self) -> String {
        format!("{}/{}", self.category, self.name)
    }
}

fn tracepoint_target(
    program: &ProgramMut<'_>,
    program_name: &str,
) -> Result<Option<TracepointTarget>, LoaderError> {
    let section = program.section().to_str().ok_or_else(|| {
        LoaderError::new(
            "attach_program",
            format!("program {program_name} has a non-UTF8 section name"),
        )
    })?;
    let Some(rest) = section
        .strip_prefix(TRACEPOINT_PREFIX)
        .or_else(|| section.strip_prefix(TP_PREFIX))
    else {
        return Ok(None);
    };
    let Some((category, name)) = rest.split_once('/') else {
        return Err(LoaderError::new(
            "attach_program",
            format!("program {program_name} has invalid tracepoint section {section}"),
        ));
    };
    if category.is_empty() || name.is_empty() {
        return Err(LoaderError::new(
            "attach_program",
            format!("program {program_name} has invalid tracepoint section {section}"),
        ));
    }
    Ok(Some(TracepointTarget {
        category: category.to_string(),
        name: name.to_string(),
    }))
}

fn attach_tracepoint(
    program: &ProgramMut<'_>,
    program_name: &str,
    target: &TracepointTarget,
    allow_missing_tracepoint: bool,
) -> Result<Option<Link>, LoaderError> {
    let tracepoint_id = match read_tracepoint_id(target) {
        Ok(tracepoint_id) => tracepoint_id,
        Err(_error) if allow_missing_tracepoint => return Ok(None),
        Err(error) => return Err(error),
    };
    let perf_event_fd = open_tracepoint_perf_event(tracepoint_id, target)?;
    let opts = PerfEventOpts {
        force_ioctl_attach: true,
        ..Default::default()
    };
    match program.attach_perf_event_with_opts(perf_event_fd, opts) {
        Ok(link) => Ok(Some(link)),
        Err(error) => {
            close_fd(perf_event_fd);
            Err(LoaderError::new(
                "attach_program",
                format!(
                    "{}; program={}; tracepoint={}; attach_mode=perf_event_ioctl; {}",
                    error,
                    program_name,
                    target.display(),
                    environment::attach_environment_description()
                ),
            ))
        }
    }
}

fn read_tracepoint_id(target: &TracepointTarget) -> Result<u64, LoaderError> {
    let roots = tracefs_roots()?;
    let mut errors = Vec::new();
    for root in roots {
        let path = root
            .join("events")
            .join(&target.category)
            .join(&target.name)
            .join("id");
        match std::fs::read_to_string(&path) {
            Ok(raw) => {
                return raw.trim().parse::<u64>().map_err(|error| {
                    LoaderError::new(
                        "tracepoint_id",
                        format!(
                            "tracepoint {} id at {} is invalid: {error}",
                            target.display(),
                            path.display()
                        ),
                    )
                });
            }
            Err(error) => errors.push(format!("{}: {error}", path.display())),
        }
    }
    Err(LoaderError::new(
        "tracepoint_id",
        format!(
            "tracepoint {} id is unavailable: {}",
            target.display(),
            errors.join("; ")
        ),
    ))
}

pub(super) fn tracefs_roots() -> Result<Vec<PathBuf>, LoaderError> {
    let mountinfo = std::fs::read_to_string("/proc/self/mountinfo").map_err(|error| {
        LoaderError::new(
            "tracefs_mount",
            format!("cannot read /proc/self/mountinfo: {error}"),
        )
    })?;
    let roots = mountinfo
        .lines()
        .filter_map(parse_tracefs_mount)
        .collect::<Vec<_>>();
    if roots.is_empty() {
        return Err(LoaderError::new(
            "tracefs_mount",
            "tracefs mount is missing",
        ));
    }
    Ok(roots)
}

fn parse_tracefs_mount(line: &str) -> Option<PathBuf> {
    let (mount_fields, fs_fields) = line.split_once(" - ")?;
    let fs_type = fs_fields.split_whitespace().next()?;
    if fs_type != "tracefs" {
        return None;
    }
    let mut fields = mount_fields.split_whitespace();
    let _mount_id = fields.next()?;
    let _parent_id = fields.next()?;
    let _device = fields.next()?;
    let _root = fields.next()?;
    fields.next().map(PathBuf::from)
}

fn open_tracepoint_perf_event(
    tracepoint_id: u64,
    target: &TracepointTarget,
) -> Result<RawFd, LoaderError> {
    let mut attr = libbpf_sys::perf_event_attr {
        type_: libbpf_sys::PERF_TYPE_TRACEPOINT,
        size: std::mem::size_of::<libbpf_sys::perf_event_attr>() as u32,
        config: tracepoint_id,
        ..Default::default()
    };
    let result = unsafe {
        libc::syscall(
            libc::SYS_perf_event_open,
            &mut attr as *mut libbpf_sys::perf_event_attr,
            -1_i32,
            0_i32,
            -1_i32,
            libbpf_sys::PERF_FLAG_FD_CLOEXEC as libc::c_ulong,
        )
    };
    if result < 0 {
        return Err(LoaderError::new(
            "perf_event_open",
            format!(
                "failed to open tracepoint {} perf event: {}; {}",
                target.display(),
                io::Error::last_os_error(),
                environment::attach_environment_description()
            ),
        ));
    }
    Ok(result as RawFd)
}

fn close_fd(fd: RawFd) {
    unsafe {
        libc::close(fd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tracefs_mount_path() {
        let line =
            "29 23 0:26 / /sys/kernel/tracing rw,nosuid,nodev,noexec,relatime - tracefs tracefs rw";
        assert_eq!(
            parse_tracefs_mount(line),
            Some(PathBuf::from("/sys/kernel/tracing"))
        );
    }

    #[test]
    fn ignores_non_tracefs_mount_path() {
        let line = "24 23 0:22 / /proc rw,nosuid,nodev,noexec,relatime - proc proc rw";
        assert_eq!(parse_tracefs_mount(line), None);
    }
}
