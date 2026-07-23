//! Seccomp BPF rule generation for launch-time user notify.

use config_core::daemon::{
    EnforcementSeccompSyscall, NetworkControlSeccompSyscall, PayloadSocketSeccompSyscall,
    PayloadTlsSeccompSyscall, ProcessSeccompSyscall,
};
use linux_platform::file_seccomp::{
    KernelFileSeccompSyscall, effective_kernel_syscalls as effective_file_syscalls,
};
use linux_platform::process_seccomp::{KernelProcessSyscall, effective_kernel_syscalls};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum SeccompRule {
    Notify(u32),
    NotifySocketPayload {
        syscall: u32,
        size_arg: u32,
        min_size: u32,
    },
    NotifyCloneProcess(u32),
    NotifyArgMasked {
        syscall: u32,
        arg: u32,
        mask: u32,
    },
}

pub(super) fn build_seccomp_rules(
    payload_tls_syscalls: Vec<PayloadTlsSeccompSyscall>,
    payload_socket_syscalls: Vec<PayloadSocketSeccompSyscall>,
    payload_socket_max_segment_bytes: u32,
    process_syscalls: Vec<ProcessSeccompSyscall>,
    network_syscalls: Vec<NetworkControlSeccompSyscall>,
    file_enforcement_syscalls: Vec<EnforcementSeccompSyscall>,
) -> Result<Vec<SeccompRule>, String> {
    let mut rules = std::collections::BTreeSet::new();
    for syscall in payload_tls_syscalls {
        rules.insert(SeccompRule::Notify(payload_tls_syscall_number(syscall)?));
    }
    for syscall in payload_socket_syscalls {
        rules.insert(payload_socket_syscall_rule(
            syscall,
            payload_socket_max_segment_bytes,
        )?);
    }
    for syscall in process_syscalls {
        let kernel_syscalls = effective_kernel_syscalls(syscall);
        if kernel_syscalls.is_empty() {
            return Err(format!(
                "process seccomp syscall {syscall:?} has no kernel syscall on this target"
            ));
        }
        for kernel_syscall in kernel_syscalls {
            rules.insert(process_seccomp_rule(kernel_syscall)?);
        }
    }
    for syscall in network_syscalls {
        rules.insert(network_control_syscall_rule(syscall)?);
    }
    for syscall in file_enforcement_syscalls {
        for kernel_syscall in effective_file_syscalls(syscall) {
            rules.insert(file_enforcement_seccomp_rule(kernel_syscall)?);
        }
    }
    Ok(rules.into_iter().collect())
}

pub(super) fn append_rule(filter: &mut Vec<libc::sock_filter>, rule: SeccompRule) {
    match rule {
        SeccompRule::Notify(syscall) => append_notify_syscall_rule(filter, syscall),
        SeccompRule::NotifySocketPayload {
            syscall,
            size_arg,
            min_size,
        } => append_socket_payload_rule(filter, syscall, size_arg, min_size),
        SeccompRule::NotifyCloneProcess(syscall) => append_clone_process_rule(filter, syscall),
        SeccompRule::NotifyArgMasked { syscall, arg, mask } => {
            append_arg_masked_notify_rule(filter, syscall, arg, mask)
        }
    }
}

fn append_notify_syscall_rule(filter: &mut Vec<libc::sock_filter>, syscall: u32) {
    filter.push(unsafe {
        libc::BPF_STMT(
            (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16,
            SECCOMP_DATA_NR_OFFSET,
        )
    });
    filter.push(unsafe {
        libc::BPF_JUMP(
            (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16,
            syscall,
            0,
            1,
        )
    });
    append_notify_return(filter);
}

fn append_socket_payload_rule(
    filter: &mut Vec<libc::sock_filter>,
    syscall: u32,
    size_arg: u32,
    min_size: u32,
) {
    filter.push(unsafe {
        libc::BPF_STMT(
            (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16,
            SECCOMP_DATA_NR_OFFSET,
        )
    });
    filter.push(unsafe {
        libc::BPF_JUMP(
            (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16,
            syscall,
            0,
            6,
        )
    });
    filter.push(unsafe {
        libc::BPF_STMT(
            (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16,
            seccomp_data_arg_high_offset(size_arg),
        )
    });
    filter.push(unsafe {
        libc::BPF_JUMP(
            (libc::BPF_JMP | libc::BPF_JGT | libc::BPF_K) as u16,
            0,
            3,
            0,
        )
    });
    filter.push(unsafe {
        libc::BPF_STMT(
            (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16,
            seccomp_data_arg_low_offset(size_arg),
        )
    });
    filter.push(unsafe {
        libc::BPF_JUMP(
            (libc::BPF_JMP | libc::BPF_JGT | libc::BPF_K) as u16,
            min_size,
            1,
            0,
        )
    });
    append_allow_return(filter);
    append_notify_return(filter);
}

fn append_clone_process_rule(filter: &mut Vec<libc::sock_filter>, syscall: u32) {
    filter.push(unsafe {
        libc::BPF_STMT(
            (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16,
            SECCOMP_DATA_NR_OFFSET,
        )
    });
    filter.push(unsafe {
        libc::BPF_JUMP(
            (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16,
            syscall,
            0,
            4,
        )
    });
    filter.push(unsafe {
        libc::BPF_STMT(
            (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16,
            SECCOMP_DATA_ARG0_LOW_OFFSET,
        )
    });
    filter.push(unsafe {
        libc::BPF_JUMP(
            (libc::BPF_JMP | libc::BPF_JSET | libc::BPF_K) as u16,
            libc::CLONE_THREAD as u32,
            0,
            1,
        )
    });
    append_allow_return(filter);
    append_notify_return(filter);
}

fn append_arg_masked_notify_rule(
    filter: &mut Vec<libc::sock_filter>,
    syscall: u32,
    arg: u32,
    mask: u32,
) {
    filter.push(unsafe {
        libc::BPF_STMT(
            (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16,
            SECCOMP_DATA_NR_OFFSET,
        )
    });
    filter.push(unsafe {
        libc::BPF_JUMP(
            (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16,
            syscall,
            0,
            3,
        )
    });
    filter.push(unsafe {
        libc::BPF_STMT(
            (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16,
            seccomp_data_arg_low_offset(arg),
        )
    });
    filter.push(unsafe {
        libc::BPF_JUMP(
            (libc::BPF_JMP | libc::BPF_JSET | libc::BPF_K) as u16,
            mask,
            0,
            1,
        )
    });
    append_notify_return(filter);
}

fn append_notify_return(filter: &mut Vec<libc::sock_filter>) {
    filter.push(unsafe {
        libc::BPF_STMT(
            (libc::BPF_RET | libc::BPF_K) as u16,
            libc::SECCOMP_RET_USER_NOTIF,
        )
    });
}

fn append_allow_return(filter: &mut Vec<libc::sock_filter>) {
    filter.push(unsafe {
        libc::BPF_STMT(
            (libc::BPF_RET | libc::BPF_K) as u16,
            libc::SECCOMP_RET_ALLOW,
        )
    });
}

fn payload_tls_syscall_number(syscall: PayloadTlsSeccompSyscall) -> Result<u32, String> {
    let raw = match syscall {
        PayloadTlsSeccompSyscall::Write => libc::SYS_write,
        PayloadTlsSeccompSyscall::Writev => libc::SYS_writev,
        PayloadTlsSeccompSyscall::Sendto => libc::SYS_sendto,
        PayloadTlsSeccompSyscall::Sendmsg => libc::SYS_sendmsg,
    };
    u32::try_from(raw).map_err(|error| format!("syscall number overflow: {error}"))
}

fn payload_socket_syscall_rule(
    syscall: PayloadSocketSeccompSyscall,
    _max_segment_bytes: u32,
) -> Result<SeccompRule, String> {
    let raw = match syscall {
        PayloadSocketSeccompSyscall::Write => libc::SYS_write,
        PayloadSocketSeccompSyscall::Writev => libc::SYS_writev,
        PayloadSocketSeccompSyscall::Sendto => libc::SYS_sendto,
        PayloadSocketSeccompSyscall::Sendmsg => libc::SYS_sendmsg,
    };
    let syscall_number =
        u32::try_from(raw).map_err(|error| format!("syscall number overflow: {error}"))?;
    Ok(match syscall {
        PayloadSocketSeccompSyscall::Write => SeccompRule::NotifySocketPayload {
            syscall: syscall_number,
            size_arg: 2,
            min_size: 0,
        },
        PayloadSocketSeccompSyscall::Sendto => SeccompRule::NotifySocketPayload {
            syscall: syscall_number,
            size_arg: 2,
            min_size: 0,
        },
        PayloadSocketSeccompSyscall::Writev | PayloadSocketSeccompSyscall::Sendmsg => {
            SeccompRule::Notify(syscall_number)
        }
    })
}

fn process_seccomp_rule(syscall: KernelProcessSyscall) -> Result<SeccompRule, String> {
    let raw = syscall.number().ok_or_else(|| {
        format!("kernel process syscall {syscall:?} is unsupported on this target")
    })?;
    let syscall_number =
        u32::try_from(raw).map_err(|error| format!("syscall number overflow: {error}"))?;
    Ok(match syscall {
        KernelProcessSyscall::Clone => SeccompRule::NotifyCloneProcess(syscall_number),
        _ => SeccompRule::Notify(syscall_number),
    })
}

fn network_control_syscall_rule(
    syscall: NetworkControlSeccompSyscall,
) -> Result<SeccompRule, String> {
    let raw = match syscall {
        NetworkControlSeccompSyscall::Connect => libc::SYS_connect,
    };
    u32::try_from(raw)
        .map(SeccompRule::Notify)
        .map_err(|error| format!("syscall number overflow: {error}"))
}

fn file_enforcement_seccomp_rule(syscall: KernelFileSeccompSyscall) -> Result<SeccompRule, String> {
    let raw = syscall
        .number()
        .ok_or_else(|| format!("file enforcement syscall {syscall:?} is unsupported"))?;
    let syscall_number =
        u32::try_from(raw).map_err(|error| format!("syscall number overflow: {error}"))?;
    Ok(match syscall {
        KernelFileSeccompSyscall::UnlinkAtRemovedir => SeccompRule::NotifyArgMasked {
            syscall: syscall_number,
            arg: 2,
            mask: libc::AT_REMOVEDIR as u32,
        },
        _ => SeccompRule::Notify(syscall_number),
    })
}

const SECCOMP_DATA_NR_OFFSET: u32 = 0;
const SECCOMP_DATA_ARG0_LOW_OFFSET: u32 = 16;
const SECCOMP_DATA_ARG_WIDTH: u32 = 8;
const SECCOMP_DATA_ARG_LOW_TO_HIGH_OFFSET: u32 = 4;

fn seccomp_data_arg_low_offset(index: u32) -> u32 {
    SECCOMP_DATA_ARG0_LOW_OFFSET + index * SECCOMP_DATA_ARG_WIDTH
}

fn seccomp_data_arg_high_offset(index: u32) -> u32 {
    seccomp_data_arg_low_offset(index) + SECCOMP_DATA_ARG_LOW_TO_HIGH_OFFSET
}
