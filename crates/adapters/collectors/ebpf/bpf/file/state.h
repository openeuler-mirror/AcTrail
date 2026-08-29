#ifndef ACTRAIL_FILE_STATE_H
#define ACTRAIL_FILE_STATE_H

#include "../abi/file_path.h"
#include "../runtime/event_transport.h"

struct actrail_file_config {
    __u32 path_max_bytes;
    __u32 event_capture_enabled;
};
struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct actrail_file_config);
} file_config SEC(".maps");

static __always_inline int file_event_capture_enabled(void) {
    __u32 key = 0;
    struct actrail_file_config *config = bpf_map_lookup_elem(&file_config, &key);

    return config && config->event_capture_enabled;
}

#endif
