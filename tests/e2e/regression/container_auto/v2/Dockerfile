ARG BASE_IMAGE=openeuler/openeuler:24.03-lts-sp3
FROM ${BASE_IMAGE}

RUN set -eux; \
    if command -v apt-get >/dev/null 2>&1; then \
        apt-get update; \
        apt-get install -y --no-install-recommends \
            ca-certificates curl libelf1 python3 tini; \
        rm -rf /var/lib/apt/lists/*; \
        init_binary=/usr/bin/tini; \
    elif command -v dnf >/dev/null 2>&1; then \
        dnf install -y \
            ca-certificates curl dumb-init elfutils-libelf python3; \
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

RUN chmod 0755 \
    /usr/local/bin/actrailctl \
    /usr/local/bin/libactrail_tls_payload_probe_sync.so

CMD ["/usr/local/bin/actrail-container-init", "--", "tail", "-f", "/dev/null"]
