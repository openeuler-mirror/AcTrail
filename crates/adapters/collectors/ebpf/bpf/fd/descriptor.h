#ifndef ACTRAIL_FD_DESCRIPTOR_H
#define ACTRAIL_FD_DESCRIPTOR_H

#include "maps.h"
#include "../runtime/endpoint.h"
#include "../runtime/event_transport.h"

static __always_inline __u32 fd_category_for_domain(__u32 domain) {
    switch (domain) {
    case AF_INET:
    case AF_INET6:
        return ACTRAIL_FD_CATEGORY_NET;
    case AF_UNIX:
        return ACTRAIL_FD_CATEGORY_IPC_UNIX_SOCKET;
    default:
        return ACTRAIL_FD_CATEGORY_NONE;
    }
}

static __always_inline struct actrail_fd_config *fd_config(void) {
    __u32 key = 0;
    return bpf_map_lookup_elem(&fd_category_config, &key);
}

static __always_inline __u32 fd_category_config_flags(void) {
    struct actrail_fd_config *config = fd_config();
    return config ? config->category_flags : 0;
}

static __always_inline int fd_tracking_enabled(void) {
    return fd_category_config_flags() != 0;
}

static __always_inline int fd_category_enabled(__u32 category) {
    return (fd_category_config_flags() & (1u << category)) != 0;
}

static __always_inline __u32 fd_slot_limit(void) {
    struct actrail_fd_config *config = fd_config();
    return config ? config->max_slots_per_process : 0;
}

static __always_inline __u32 fd_process_active_count(__u32 pid) {
    __u32 *count = bpf_map_lookup_elem(&fd_process_active_counts, &pid);
    return count ? *count : 0;
}

static __always_inline int fd_process_active_count_seed_empty(__u32 pid) {
    __u32 count = 0;

    if (!pid) {
        return 0;
    }
    return bpf_map_update_elem(&fd_process_active_counts, &pid, &count, BPF_NOEXIST) == 0;
}

static __always_inline int fd_process_active_count_acquire(__u32 pid) {
    __u32 *count = bpf_map_lookup_elem(&fd_process_active_counts, &pid);
    __u32 initial = 1;

    if (count) {
        __sync_fetch_and_add(count, 1);
        return 1;
    }
    if (bpf_map_update_elem(&fd_process_active_counts, &pid, &initial, BPF_NOEXIST) == 0) {
        return 1;
    }
    count = bpf_map_lookup_elem(&fd_process_active_counts, &pid);
    if (!count) {
        return 0;
    }
    __sync_fetch_and_add(count, 1);
    return 1;
}

static __always_inline void fd_process_active_count_release(__u32 pid) {
    __u32 *count = bpf_map_lookup_elem(&fd_process_active_counts, &pid);

    if (count && *count) {
        __sync_fetch_and_add(count, (__u32)-1);
    }
}

static __always_inline struct actrail_fd_state *fd_lookup(__u32 pid, __u32 fd) {
    struct actrail_fd_key key = { .pid = pid, .fd = fd };
    return bpf_map_lookup_elem(&fd_table, &key);
}

static __always_inline __u64 fd_kernel_file_identity(__u32 fd) {
    struct task_struct *task = actrail_bpf_get_current_task();
    struct files_struct *files = 0;
    struct fdtable *table = 0;
    struct file **entries = 0;
    struct file *file = 0;
    __u32 max_fds = 0;
    __u64 entry_address;

    if (!task || ACTRAIL_CORE_READ(&files, task, files) != 0 || !files
        || ACTRAIL_CORE_READ(&table, files, fdt) != 0 || !table
        || ACTRAIL_CORE_READ(&max_fds, table, max_fds) != 0) {
        return ACTRAIL_FD_FILE_IDENTITY_READ_FAILED;
    }
    if (fd >= max_fds) {
        return 0;
    }
    if (ACTRAIL_CORE_READ(&entries, table, fd) != 0 || !entries) {
        return ACTRAIL_FD_FILE_IDENTITY_READ_FAILED;
    }
    entry_address = (__u64)entries + ((__u64)fd * sizeof(*entries));
    if (bpf_probe_read_kernel(
            &file,
            sizeof(file),
            (const void *)entry_address
        ) != 0) {
        return ACTRAIL_FD_FILE_IDENTITY_READ_FAILED;
    }
    return (__u64)file;
}

static __always_inline int fd_kernel_cloexec(__u32 fd) {
    struct task_struct *task = actrail_bpf_get_current_task();
    struct files_struct *files = 0;
    struct fdtable *table = 0;
    unsigned long *bitmap = 0;
    unsigned long word = 0;
    __u32 max_fds = 0;
    __u32 bits_per_word = sizeof(word) * 8;
    __u64 word_address;

    if (!task || ACTRAIL_CORE_READ(&files, task, files) != 0 || !files
        || ACTRAIL_CORE_READ(&table, files, fdt) != 0 || !table
        || ACTRAIL_CORE_READ(&max_fds, table, max_fds) != 0) {
        return -1;
    }
    if (fd >= max_fds) {
        return 0;
    }
    if (ACTRAIL_CORE_READ(&bitmap, table, close_on_exec) != 0 || !bitmap) {
        return -1;
    }
    word_address = (__u64)bitmap + ((__u64)(fd / bits_per_word) * sizeof(word));
    if (bpf_probe_read_kernel(
            &word,
            sizeof(word),
            (const void *)word_address
        ) != 0) {
        return -1;
    }
    return (word >> (fd % bits_per_word)) & 1;
}

#endif
