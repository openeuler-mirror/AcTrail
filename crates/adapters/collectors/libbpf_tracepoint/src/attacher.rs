use std::io;
use std::os::fd::RawFd;
use std::path::PathBuf;
use std::sync::OnceLock;

use libbpf_rs::{Link, PerfEventOpts, ProgramMut, libbpf_sys};

use crate::TracepointAttachError;

const TRACEPOINT_PREFIX: &str = "tracepoint/";
const TP_PREFIX: &str = "tp/";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TracepointRequirement {
    Required,
    Optional,
}

pub enum TracepointAttachOutcome {
    NotTracepoint,
    Unavailable,
    Attached(Link),
}

pub struct TracepointProgramAttacher {
    tracefs: OnceLock<Result<Tracefs, TracepointAttachError>>,
}

impl TracepointProgramAttacher {
    pub const fn new() -> Self {
        Self {
            tracefs: OnceLock::new(),
        }
    }

    pub fn attach(
        &self,
        program: &ProgramMut<'_>,
        program_name: &str,
        requirement: TracepointRequirement,
    ) -> Result<TracepointAttachOutcome, TracepointAttachError> {
        let Some(target) = TracepointTarget::from_program(program, program_name)? else {
            return Ok(TracepointAttachOutcome::NotTracepoint);
        };
        let tracepoint_id = match self.read_tracepoint_id(&target) {
            Ok(tracepoint_id) => tracepoint_id,
            Err(_) if requirement == TracepointRequirement::Optional => {
                return Ok(TracepointAttachOutcome::Unavailable);
            }
            Err(error) => return Err(error),
        };
        let perf_event_fd = open_tracepoint_perf_event(tracepoint_id, &target)?;
        let opts = PerfEventOpts {
            force_ioctl_attach: true,
            ..Default::default()
        };
        match program.attach_perf_event_with_opts(perf_event_fd, opts) {
            Ok(link) => Ok(TracepointAttachOutcome::Attached(link)),
            Err(error) => {
                close_fd(perf_event_fd);
                Err(TracepointAttachError::new(
                    "attach_program",
                    format!(
                        "{error}; program={program_name}; tracepoint={}; \
                         attach_mode=perf_event_ioctl",
                        target.display()
                    ),
                ))
            }
        }
    }

    fn read_tracepoint_id(&self, target: &TracepointTarget) -> Result<u64, TracepointAttachError> {
        self.tracefs
            .get_or_init(Tracefs::discover)
            .as_ref()
            .map_err(Clone::clone)?
            .read_tracepoint_id(target)
    }
}

impl Default for TracepointProgramAttacher {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Tracefs {
    roots: Vec<PathBuf>,
}

impl Tracefs {
    fn discover() -> Result<Self, TracepointAttachError> {
        let mountinfo = std::fs::read_to_string("/proc/self/mountinfo").map_err(|error| {
            TracepointAttachError::new(
                "tracefs_mount",
                format!("cannot read /proc/self/mountinfo: {error}"),
            )
        })?;
        let roots = mountinfo
            .lines()
            .filter_map(parse_tracefs_mount)
            .collect::<Vec<_>>();
        if roots.is_empty() {
            return Err(TracepointAttachError::new(
                "tracefs_mount",
                "tracefs mount is missing",
            ));
        }
        Ok(Self { roots })
    }

    fn read_tracepoint_id(&self, target: &TracepointTarget) -> Result<u64, TracepointAttachError> {
        let mut errors = Vec::new();
        for root in &self.roots {
            let path = root
                .join("events")
                .join(&target.category)
                .join(&target.name)
                .join("id");
            match std::fs::read_to_string(&path) {
                Ok(raw) => {
                    return raw.trim().parse::<u64>().map_err(|error| {
                        TracepointAttachError::new(
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
        Err(TracepointAttachError::new(
            "tracepoint_id",
            format!(
                "tracepoint {} id is unavailable: {}",
                target.display(),
                errors.join("; ")
            ),
        ))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TracepointTarget {
    category: String,
    name: String,
}

impl TracepointTarget {
    fn from_program(
        program: &ProgramMut<'_>,
        program_name: &str,
    ) -> Result<Option<Self>, TracepointAttachError> {
        let section = program.section().to_str().ok_or_else(|| {
            TracepointAttachError::new(
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
            return Err(invalid_tracepoint_section(program_name, section));
        };
        if category.is_empty() || name.is_empty() {
            return Err(invalid_tracepoint_section(program_name, section));
        }
        Ok(Some(Self {
            category: category.to_string(),
            name: name.to_string(),
        }))
    }

    fn display(&self) -> String {
        format!("{}/{}", self.category, self.name)
    }
}

fn invalid_tracepoint_section(program_name: &str, section: &str) -> TracepointAttachError {
    TracepointAttachError::new(
        "attach_program",
        format!("program {program_name} has invalid tracepoint section {section}"),
    )
}

fn parse_tracefs_mount(line: &str) -> Option<PathBuf> {
    let (mount_fields, fs_fields) = line.split_once(" - ")?;
    if fs_fields.split_whitespace().next()? != "tracefs" {
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
) -> Result<RawFd, TracepointAttachError> {
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
        return Err(TracepointAttachError::new(
            "perf_event_open",
            format!(
                "failed to open tracepoint {} perf event: {}",
                target.display(),
                io::Error::last_os_error()
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
