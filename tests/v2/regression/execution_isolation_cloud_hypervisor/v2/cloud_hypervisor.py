from __future__ import annotations

import stat
import time
from dataclasses import dataclass
from pathlib import Path
from typing import ClassVar


_UNIX_SOCKET_PATH_LIMIT = 107


@dataclass(frozen=True)
class HybridVsockSocketInventory:
    """Identifies the hybrid-vsock base socket owned by one newly-created VM."""

    vm_root: Path
    SOCKET_PATTERN: ClassVar[str]
    DISPLAY: ClassVar[str]
    GATEWAY_USES_BASE_SOCKET: ClassVar[bool]

    def snapshot(self) -> frozenset[Path]:
        if not self.vm_root.is_dir():
            return frozenset()
        sockets: set[Path] = set()
        for candidate in self.vm_root.glob(self.SOCKET_PATTERN):
            try:
                mode = candidate.stat().st_mode
            except OSError:
                continue
            if stat.S_ISSOCK(mode):
                sockets.add(candidate.resolve())
        return frozenset(sockets)

    def resolve_new_base_socket(
        self,
        before: frozenset[Path],
        after: frozenset[Path],
    ) -> Path:
        created = sorted(after - before)
        if len(created) != 1:
            rendered = ", ".join(str(path) for path in created) or "<none>"
            raise RuntimeError(
                f"expected exactly one test-owned {self.DISPLAY} socket; "
                f"found {len(created)}: {rendered}"
            )
        base = created[0]
        try:
            base.relative_to(self.vm_root.resolve())
        except ValueError as error:
            raise RuntimeError(
                f"{self.DISPLAY} socket escaped VM root: {base}"
            ) from error
        return base

    def wait_new_base_socket(
        self,
        before: frozenset[Path],
        timeout_seconds: float,
    ) -> Path:
        if timeout_seconds <= 0:
            raise ValueError("hybrid VSOCK discovery timeout must be positive")
        deadline = time.monotonic() + timeout_seconds
        after = self.snapshot()
        while time.monotonic() < deadline:
            created = after - before
            if created:
                return self.resolve_new_base_socket(before, after)
            time.sleep(0.05)
            after = self.snapshot()
        return self.resolve_new_base_socket(before, after)

    def gateway_socket(self, base: Path, port: int) -> Path:
        if port < 1027 or port > 65535:
            raise ValueError(
                f"{self.DISPLAY} execution-isolation VSOCK port must be "
                "1027..65535"
            )
        endpoint = self.listener_socket(base, port)
        if len(str(endpoint).encode()) > _UNIX_SOCKET_PATH_LIMIT:
            raise RuntimeError(
                f"{self.DISPLAY} VSOCK port-suffix path exceeds the UNIX "
                f"socket limit: {endpoint}"
            )
        if endpoint.exists() or endpoint.is_symlink():
            raise RuntimeError(
                f"refusing to replace existing VSOCK endpoint: {endpoint}"
            )
        return base if self.GATEWAY_USES_BASE_SOCKET else endpoint

    def wait_listener_socket(
        self,
        base: Path,
        port: int,
        timeout_seconds: float,
    ) -> Path:
        if timeout_seconds <= 0:
            raise ValueError("hybrid VSOCK listener timeout must be positive")
        endpoint = self.listener_socket(base, port)
        deadline = time.monotonic() + timeout_seconds
        while time.monotonic() < deadline:
            try:
                mode = endpoint.lstat().st_mode
            except FileNotFoundError:
                time.sleep(0.05)
                continue
            if stat.S_ISSOCK(mode):
                return endpoint
            raise RuntimeError(
                f"{self.DISPLAY} VSOCK listener is not a UNIX socket: {endpoint}"
            )
        raise RuntimeError(
            f"timed out waiting for {self.DISPLAY} VSOCK listener: {endpoint}"
        )

    @staticmethod
    def listener_socket(base: Path, port: int) -> Path:
        return Path(f"{base}_{port}")


class CloudHypervisorSocketInventory(HybridVsockSocketInventory):
    """Identifies the Cloud Hypervisor socket owned by one newly-created VM."""

    SOCKET_PATTERN = "*/clh.sock"
    DISPLAY = "Cloud Hypervisor"
    GATEWAY_USES_BASE_SOCKET = False


class FirecrackerSocketInventory(HybridVsockSocketInventory):
    """Identifies Kata Firecracker's per-sandbox hybrid-vsock base UDS."""

    SOCKET_PATTERN = "*/root/kata.hvsock"
    DISPLAY = "Firecracker"
    GATEWAY_USES_BASE_SOCKET = True
