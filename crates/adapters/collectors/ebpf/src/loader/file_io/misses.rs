//! Recursion losses for the file object lifetime and I/O tracing programs.

use std::ffi::OsStr;
use std::mem::{offset_of, size_of};
use std::os::fd::{AsFd, AsRawFd, OwnedFd};

use libbpf_rs::{Object, libbpf_sys};

use crate::loader::LoaderError;

const PROGRAMS: [&str; 6] = [
    "handle_file_io_read",
    "handle_file_io_readv",
    "handle_file_io_write",
    "handle_file_io_writev",
    "handle_file_io_permission",
    "handle_file_io_free",
];
const FREE_PROGRAM: &str = "handle_file_io_free";

pub(super) struct FileIoProgramMissDelta {
    pub program: &'static str,
    pub count: u64,
    pub identity_untrusted: bool,
}

struct ProgramCounter {
    name: &'static str,
    fd: OwnedFd,
    delivered: u64,
}

impl ProgramCounter {
    fn read(&self) -> Result<u64, LoaderError> {
        // The UAPI structure contains only integer fields and user pointers.
        // All output-array pointers remain null, so no instruction/BTF buffers
        // are allocated or fetched by this diagnostic query.
        let mut info: libbpf_sys::bpf_prog_info = unsafe { std::mem::zeroed() };
        let mut length = size_of::<libbpf_sys::bpf_prog_info>() as u32;
        let result = unsafe {
            libbpf_sys::bpf_prog_get_info_by_fd(self.fd.as_raw_fd(), &mut info, &mut length)
        };
        if result != 0 {
            return Err(LoaderError::new(
                "file_io_program_misses",
                format!(
                    "cannot read {} recursion losses: {} (libbpf result {result})",
                    self.name,
                    std::io::Error::last_os_error(),
                ),
            ));
        }
        let required = offset_of!(libbpf_sys::bpf_prog_info, recursion_misses) + size_of::<u64>();
        if (length as usize) < required {
            return Err(LoaderError::new(
                "file_io_program_misses",
                format!(
                    "kernel program info for {} is {length} bytes; recursion_misses requires {required}",
                    self.name,
                ),
            ));
        }
        Ok(info.recursion_misses)
    }
}

pub(super) struct FileIoProgramMisses {
    programs: Vec<ProgramCounter>,
    identity_untrusted: bool,
}

impl FileIoProgramMisses {
    /// Call only when the six file I/O programs have been loaded.
    pub(super) fn from_object(object: &Object) -> Result<Self, LoaderError> {
        let mut programs = Vec::with_capacity(PROGRAMS.len());
        let mut identity_untrusted = false;
        for name in PROGRAMS {
            let program = object
                .progs()
                .find(|program| program.name() == OsStr::new(name))
                .ok_or_else(|| {
                    LoaderError::new(
                        "file_io_program_misses",
                        format!("required file I/O program {name} is absent"),
                    )
                })?;
            let fd = program.as_fd().try_clone_to_owned().map_err(|error| {
                LoaderError::new(
                    "file_io_program_misses",
                    format!("cannot retain {name} program descriptor: {error}"),
                )
            })?;
            let counter = ProgramCounter {
                name,
                fd,
                delivered: 0,
            };
            let current = counter.read()?;
            identity_untrusted |= name == FREE_PROGRAM && current != 0;
            // Losses during startup still belong in the first diagnostic poll.
            programs.push(counter);
        }
        Ok(Self {
            programs,
            identity_untrusted,
        })
    }

    pub(super) fn identity_untrusted(&self) -> bool {
        self.identity_untrusted
    }

    /// The caller schedules this with the file summary checkpoint interval.
    /// Any lifetime-hook miss invalidates identity trust for this runtime.
    pub(super) fn poll(&mut self) -> Result<Vec<FileIoProgramMissDelta>, LoaderError> {
        let mut current = Vec::with_capacity(self.programs.len());
        let mut first_error = None;
        for program in &self.programs {
            match program.read() {
                Ok(count) if count >= program.delivered => {
                    self.identity_untrusted |= program.name == FREE_PROGRAM && count != 0;
                    current.push(count);
                }
                Ok(count) => {
                    self.identity_untrusted |= program.name == FREE_PROGRAM;
                    first_error.get_or_insert_with(|| {
                        LoaderError::new(
                            "file_io_program_misses",
                            format!(
                                "{} recursion counter regressed from {} to {count}",
                                program.name, program.delivered,
                            ),
                        )
                    });
                }
                Err(error) => {
                    self.identity_untrusted |= program.name == FREE_PROGRAM;
                    first_error.get_or_insert(error);
                }
            }
        }
        if let Some(error) = first_error {
            // Keep checkpoints intact so later successful reads report losses.
            return Err(error);
        }
        let mut changes = Vec::new();
        for (program, count) in self.programs.iter_mut().zip(current) {
            if count != program.delivered {
                changes.push(FileIoProgramMissDelta {
                    program: program.name,
                    count: count - program.delivered,
                    identity_untrusted: self.identity_untrusted,
                });
            }
            program.delivered = count;
        }
        Ok(changes)
    }
}
