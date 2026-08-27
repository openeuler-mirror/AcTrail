# AcTrail agent container — automatic host-eBPF × seccomp-notify selection.
#
# deploy.sh prepares a minimal context containing version-matched actrailctl and
# TLS probe binaries. BASE_IMAGE may be openEuler/dnf or Ubuntu/apt.
#
# Run with Docker default seccomp, or add AcTrail's profile when notify is
# available:
#   --security-opt seccomp="$(pwd)/deploy/container-auto/seccomp/actrail-notify.json"
ARG BASE_IMAGE=openeuler/openeuler:24.03-lts-sp3
FROM ${BASE_IMAGE}

RUN set -eux; \
    if command -v apt-get >/dev/null 2>&1; then \
        apt-get update; \
        apt-get install -y --no-install-recommends tini libelf1 ca-certificates; \
        rm -rf /var/lib/apt/lists/*; \
        init_binary=/usr/bin/tini; \
    elif command -v dnf >/dev/null 2>&1; then \
        dnf install -y dumb-init elfutils-libelf ca-certificates; \
        dnf clean all; \
        rm -rf /var/cache/dnf; \
        init_binary=/usr/bin/dumb-init; \
    else \
        echo 'unsupported base image: apt-get or dnf is required' >&2; \
        exit 1; \
    fi; \
    test -x "${init_binary}"; \
    ln -s "${init_binary}" /usr/local/bin/actrail-container-init; \
    test -x /usr/local/bin/actrail-container-init

COPY actrailctl /usr/local/bin/actrailctl
COPY libactrail_tls_payload_probe_sync.so /usr/local/bin/libactrail_tls_payload_probe_sync.so
RUN chmod 0755 /usr/local/bin/actrailctl /usr/local/bin/libactrail_tls_payload_probe_sync.so

ENTRYPOINT ["/usr/local/bin/actrail-container-init", "--", \
    "actrailctl", "--config", "/etc/actrail/container-auto.conf", \
    "launch", "--host-ebpf", "auto", "--seccomp-notify", "auto", "--"]
CMD ["/bin/sh", "-c", "echo 'pass the agent command as docker run args' && exit 1"]
