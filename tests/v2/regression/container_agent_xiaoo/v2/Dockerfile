ARG BASE_IMAGE=ubuntu:24.04
FROM ${BASE_IMAGE}

COPY actrailctl /usr/local/bin/actrailctl
COPY libactrail_tls_payload_probe_sync.so /usr/local/bin/libactrail_tls_payload_probe_sync.so
COPY xiaoo /root/.cargo/bin/xiaoo
COPY workload.sh /usr/local/bin/actrail-multi-workload

RUN chmod 0755 \
    /usr/local/bin/actrailctl \
    /usr/local/bin/libactrail_tls_payload_probe_sync.so \
    /root/.cargo/bin/xiaoo \
    /usr/local/bin/actrail-multi-workload

CMD ["tail", "-f", "/dev/null"]
