/* Combined entry retained while runtime migrates to physical nodes. */
#define ACTRAIL_COMBINED_OBJECT
#include "units/fd/io.bpf.c"
#include "units/fd/lifecycle.bpf.c"
#include "units/file/io.bpf.c"
#include "units/file/mutation.bpf.c"
#include "units/file/open.bpf.c"
#include "units/network/transport.bpf.c"
#include "units/process/context.bpf.c"
#include "units/process/mapping.bpf.c"
#include "units/process/observation.bpf.c"
#include "units/tls/gnutls.bpf.c"
#include "units/tls/go.bpf.c"
#include "units/tls/nss.bpf.c"
#include "units/tls/rustls.bpf.c"
#include "units/tls/ssl.bpf.c"

char LICENSE[] SEC("license") = "GPL";
