ARG BASE_IMAGE=ubuntu:24.04
FROM ${BASE_IMAGE}

RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends \
        ca-certificates curl libelf1 python3 tini; \
    rm -rf /var/lib/apt/lists/*

COPY actrailctl /usr/local/bin/actrailctl
COPY libactrail_tls_payload_probe_sync.so /usr/local/bin/libactrail_tls_payload_probe_sync.so

RUN chmod 0755 \
    /usr/local/bin/actrailctl \
    /usr/local/bin/libactrail_tls_payload_probe_sync.so

CMD ["tail", "-f", "/dev/null"]
