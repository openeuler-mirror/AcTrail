/* Shared domain definitions; BPF entry programs reside in units/. */
#ifndef ACTRAIL_PROCESS_MAPPING_H
#define ACTRAIL_PROCESS_MAPPING_H

#include "../common/uprobe_registers.h"
#include "observe.h"

enum actrail_mapping_flags {
    ACTRAIL_VM_EXEC = 4,
};

#endif
