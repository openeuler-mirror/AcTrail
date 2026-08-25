from __future__ import annotations

import time
from dataclasses import dataclass

from tests.v2.common.kata_runtime import GuestConsole, KataTestContainer

from ..config import CloudHypervisorExecutionIsolationConfig


@dataclass(frozen=True)
class GuestSystemSandboxObserver:
    """Controls the root Guest observer without starting a workload-local daemon."""

    guest: GuestConsole
    config: CloudHypervisorExecutionIsolationConfig

    CONTROL_SOCKET = "/dev/actrail/sandbox-observer-control.sock"
    READY_MARKER = "/dev/actrail/sandbox-observer.ready"
    LOG_PATH = "/dev/actrail/sandbox-observer.log"
    CONNECT_UNIT = "actrail-sb-connect.service"

    def require_ready_and_unconnected(self, vm: KataTestContainer) -> None:
        expected_ready = (
            "actrail-sb daemon ready "
            f"control_socket={self.CONTROL_SOCKET} "
            "connected=false publication_enabled=false"
        )
        poll_iterations = max(
            1,
            (self.config.ready_timeout_seconds - 5) * 2,
        )
        command = (
            f"remaining={poll_iterations}; "
            "while [ $remaining -gt 0 ]; do "
            f"if test -S {self.CONTROL_SOCKET} "
            f"&& test -f {self.READY_MARKER} "
            f"&& test -f /usr/lib/systemd/system/{self.CONNECT_UNIT} "
            f"&& grep -Fqx '{expected_ready}' {self.LOG_PATH} "
            f"&& ! systemctl is-enabled --quiet {self.CONNECT_UNIT} "
            f"&& ! systemctl is-active --quiet {self.CONNECT_UNIT}; then "
            "exit 0; fi; remaining=$((remaining - 1)); sleep 0.5; done; "
            "systemctl --no-pager --full status actrail-sb.service || true; "
            f"systemctl --no-pager --full status {self.CONNECT_UNIT} || true; "
            "ls -la /dev/actrail || true; "
            f"tail -n 80 {self.LOG_PATH} || true; exit 72"
        )
        result = self.guest.capture(
            vm.container_id,
            command,
            timeout=self.config.ready_timeout_seconds + 2,
        )
        if result.returncode != 0:
            raise RuntimeError(
                self.config.IDENTITY.failure(
                    "Guest system sandbox observer was not ready and unconnected: "
                    + (result.diagnostic or f"exit={result.returncode}")
                )
            )

    def connect(self, vm: KataTestContainer) -> None:
        deadline = time.monotonic() + self.config.ready_timeout_seconds
        last_diagnostic = "no connection attempt completed"
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                break
            request_timeout_ms = max(
                100,
                min(5_000, int(max(0.1, remaining - 0.5) * 1_000)),
            )
            result = self.guest.capture(
                vm.container_id,
                "LD_LIBRARY_PATH=/usr/local/lib/actrail "
                "/usr/local/bin/actrail-sb connect "
                f"--control-socket {self.CONTROL_SOCKET} "
                f"--host-cid {self.config.vsock_host_cid} "
                f"--port {self.config.vsock_port} "
                f"--request-timeout-ms {request_timeout_ms}",
                timeout=min(remaining, request_timeout_ms / 1_000 + 2),
            )
            output = result.stdout + result.stderr
            if result.returncode == 0:
                if "actrail-sb connected sb_id=" not in output:
                    raise RuntimeError(
                        self.config.IDENTITY.failure(
                            "Guest system sandbox observer connect omitted "
                            "handshake evidence"
                        )
                    )
                return
            last_diagnostic = result.diagnostic or f"exit={result.returncode}"
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                break
            time.sleep(min(0.5, remaining))
        raise RuntimeError(
            self.config.IDENTITY.failure(
                "Guest system sandbox observer VSOCK connection failed before "
                f"the readiness deadline: {last_diagnostic}"
            )
        )
