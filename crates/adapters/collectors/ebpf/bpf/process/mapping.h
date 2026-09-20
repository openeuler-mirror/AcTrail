#ifndef ACTRAIL_PROCESS_MAPPING_H
#define ACTRAIL_PROCESS_MAPPING_H

#include "../common/uprobe_registers.h"
#include "observe.h"

enum actrail_mapping_flags {
    ACTRAIL_VM_EXEC = 4,
};

SEC("kprobe/perf_event_mmap")
int handle_tls_mapping(struct pt_regs *ctx) {
    __u32 pid = 0, tid = 0, lookup_flags = 0;
    __u64 *trace = lookup_current_trace(&pid, &tid, &lookup_flags);
    if (!trace || !process_observation_is_detailed(current_kernel_tgid())) return 0;
    __u64 trace_id = *trace;
    struct actrail_process_exec_config *config = current_process_exec_config();
    if (!config || !config->executable_identity_enabled) return 0;

    struct vm_area_struct *vma = (void *)(unsigned long)ACTRAIL_UPROBE_ARG1(ctx);
    struct task_struct *task = actrail_bpf_get_current_task();
    struct mm_struct *current_mm = 0, *mapping_mm = 0;
    struct file *file = 0;
    unsigned long flags = 0, start = 0, end = 0;
    if (!vma || ACTRAIL_CORE_READ(&flags, vma, vm_flags) || !(flags & ACTRAIL_VM_EXEC) ||
        ACTRAIL_CORE_READ(&file, vma, vm_file) || !file ||
        ACTRAIL_CORE_READ(&mapping_mm, vma, vm_mm) || !mapping_mm ||
        ACTRAIL_CORE_READ(&current_mm, task, mm) || current_mm != mapping_mm ||
        ACTRAIL_CORE_READ(&start, vma, vm_start) ||
        ACTRAIL_CORE_READ(&end, vma, vm_end) || start >= end) return 0;

    struct actrail_tls_mapping_event *event = actrail_event_reserve(sizeof(*event));
    if (!event) return 0;
    __builtin_memset(event, 0, sizeof(*event));
    capture_file_identity(file, &event->file_identity);
    if (!event->file_identity.valid) {
        actrail_event_discard(event);
        return 0;
    }
    init_current_event_header(&event->header, ACTRAIL_TLS_MAPPING, sizeof(*event), trace_id);
    event->start = start;
    event->end = end;
    actrail_event_submit(ctx, event);
    return 0;
}

#endif
