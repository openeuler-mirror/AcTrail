from __future__ import annotations

import stat
from dataclasses import dataclass
from pathlib import Path


_UNIX_SOCKET_PATH_LIMIT = 107


@dataclass(frozen=True)
class CloudHypervisorSocketInventory:
    """Identifies the Cloud Hypervisor socket owned by one newly-created VM."""

    vm_root: Path

    def snapshot(self) -> frozenset[Path]:
        if not self.vm_root.is_dir():
            return frozenset()
        sockets: set[Path] = set()
        for candidate in self.vm_root.glob("*/clh.sock"):
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
                "expected exactly one test-owned Cloud Hypervisor socket; "
                f"found {len(created)}: {rendered}"
            )
        base = created[0]
        try:
            base.relative_to(self.vm_root.resolve())
        except ValueError as error:
            raise RuntimeError(
                f"Cloud Hypervisor socket escaped VM root: {base}"
            ) from error
        return base

    def gateway_socket(self, base: Path, port: int) -> Path:
        if port < 1027 or port > 65535:
            raise ValueError("execution-isolation VSOCK port must be 1027..65535")
        socket_path = Path(f"{base}_{port}")
        if len(str(socket_path).encode()) > _UNIX_SOCKET_PATH_LIMIT:
            raise RuntimeError(
                "Cloud Hypervisor VSOCK port-suffix path exceeds the UNIX "
                f"socket limit: {socket_path}"
            )
        if socket_path.exists() or socket_path.is_symlink():
            raise RuntimeError(
                f"refusing to replace existing VSOCK endpoint: {socket_path}"
            )
        return socket_path
