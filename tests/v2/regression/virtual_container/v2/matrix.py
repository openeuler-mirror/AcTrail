from __future__ import annotations

from dataclasses import dataclass


TRACE_TITLE_TOKEN = "@TRACE_TITLE@"


@dataclass(frozen=True)
class InterfaceCase:
    name: str
    gid_delta: int
    command: tuple[str, ...]
    expected_markers: tuple[str, ...]
    expect_failure: bool = False
    refreshable_failure: bool = False


@dataclass(frozen=True)
class DataCase:
    name: str
    needs_tls: bool
    needs_ebpf: bool
    expected_markers: tuple[str, ...]


INTERFACE_CASES = (
    InterfaceCase(
        name="verify",
        gid_delta=0,
        command=("/opt/actrail/bin/verify-interface",),
        expected_markers=("ACTRAIL_WORKLOAD_INTERFACE_OK",),
    ),
    InterfaceCase(
        name="deny",
        gid_delta=1,
        command=("/opt/actrail/bin/verify-interface",),
        expected_markers=("missing supplemental GID",),
        expect_failure=True,
    ),
    InterfaceCase(
        name="launch",
        gid_delta=0,
        command=(
            "/bin/sh",
            "/opt/actrail/bin/actrail-init",
            "--name",
            TRACE_TITLE_TOKEN,
            "--",
            "/bin/true",
        ),
        expected_markers=(
            "deployment_permissions_selected=",
            "trace ",
        ),
    ),
    InterfaceCase(
        name="namespace",
        gid_delta=0,
        command=(
            "/bin/sh",
            "/opt/actrail/bin/actrail-init",
            "--name",
            TRACE_TITLE_TOKEN,
            "--",
            "/opt/actrail-test/assert-pid-namespace",
        ),
        expected_markers=("ACTRAIL_PID_NAMESPACE_OK",),
    ),
)


DATA_CASES = (
    DataCase(
        name="tls-only",
        needs_tls=True,
        needs_ebpf=False,
        expected_markers=(
            "TlsUserSpace",
            "SSL_write",
            "SSL_read",
        ),
    ),
    DataCase(
        name="ebpf-only",
        needs_tls=False,
        needs_ebpf=True,
        expected_markers=(
            "host_ebpf:enabled",
            "events",
            "network_events",
        ),
    ),
    DataCase(
        name="combo",
        needs_tls=True,
        needs_ebpf=True,
        expected_markers=(
            "host_ebpf:enabled",
            "TlsUserSpace",
            "SSL_write",
            "SSL_read",
        ),
    ),
)
