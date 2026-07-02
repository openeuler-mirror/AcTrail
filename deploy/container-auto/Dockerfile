# AcTrail agent container — automatic host-eBPF × seccomp-notify selection.
#
# Build after copying version-matched actrailctl and TLS probe library:
#   docker build -f deploy/container-auto/Dockerfile \
#     -t actrail-agent:auto deploy/container-auto/
#
# Run with Docker default seccomp, or add AcTrail's profile when notify is
# available:
#   --security-opt seccomp="$(pwd)/deploy/container-auto/seccomp/actrail-notify.json"
ARG BASE_IMAGE=ubuntu:24.04
FROM ${BASE_IMAGE}

RUN set -eux; \
    if command -v apt-get >/dev/null 2>&1; then \
        apt-get update; \
        apt-get install -y --no-install-recommends tini libelf1 ca-certificates; \
        rm -rf /var/lib/apt/lists/*; \
    fi

COPY actrailctl /usr/local/bin/actrailctl
COPY libactrail_tls_payload_probe_sync.so /usr/local/bin/libactrail_tls_payload_probe_sync.so
RUN chmod 0755 /usr/local/bin/actrailctl /usr/local/bin/libactrail_tls_payload_probe_sync.so

ENTRYPOINT ["tini", "--", \
    "actrailctl", "--config", "/etc/actrail/container-auto.conf", \
    "launch", "--host-ebpf", "auto", "--seccomp-notify", "auto", "--"]
CMD ["/bin/sh", "-c", "echo 'pass the agent command as docker run args' && exit 1"]
