//! NSS NSPR plaintext I/O entry and completion programs.

use super::TlsUprobeTarget;

pub(in crate::loader::tls) const NSS_NSPR_UPROBE_TARGETS: &[TlsUprobeTarget] = &[
    TlsUprobeTarget {
        program: "handle_nspr_pr_write_enter",
        symbol: "PR_Write",
        retprobe: false,
    },
    TlsUprobeTarget {
        program: "handle_nspr_pr_write_exit",
        symbol: "PR_Write",
        retprobe: true,
    },
    TlsUprobeTarget {
        program: "handle_nspr_pr_send_enter",
        symbol: "PR_Send",
        retprobe: false,
    },
    TlsUprobeTarget {
        program: "handle_nspr_pr_send_exit",
        symbol: "PR_Send",
        retprobe: true,
    },
    TlsUprobeTarget {
        program: "handle_nspr_pr_read_enter",
        symbol: "PR_Read",
        retprobe: false,
    },
    TlsUprobeTarget {
        program: "handle_nspr_pr_read_exit",
        symbol: "PR_Read",
        retprobe: true,
    },
    TlsUprobeTarget {
        program: "handle_nspr_pr_recv_enter",
        symbol: "PR_Recv",
        retprobe: false,
    },
    TlsUprobeTarget {
        program: "handle_nspr_pr_recv_exit",
        symbol: "PR_Recv",
        retprobe: true,
    },
];
