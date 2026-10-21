from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class KataRuntimeDefaults:
    cleanup_timeout_seconds: float = 5.0
    cleanup_poll_seconds: float = 0.1
    namespace: str = "default"
    image_timeout_seconds: float = 600.0
    ready_timeout_seconds: float = 60.0
    read_only_mounts: bool = True
    privileged_without_host_devices: bool = False
    archive_copy_chunk_bytes: int = 1024 * 1024
    checksum_read_chunk_bytes: int = 1024 * 1024
    kata_runtime: str = "kata-runtime"
    script_command: str = "script"
    ctr_command: str = "ctr"
    capability_timeout_seconds: float = 10.0
    manifest_name: str = "MANIFEST.sha256"
    artifact_timeout_seconds: float = 600.0
    require_xiaoo: bool = False
    require_preinstalled_xiaoo: bool = False
    require_sandbox_observer: bool = False


DEFAULTS = KataRuntimeDefaults()
