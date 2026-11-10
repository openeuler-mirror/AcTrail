"""Runtime parameters for the Btrfs checkpoint acceptance."""
from dataclasses import dataclass


@dataclass(frozen=True)
class CheckpointBtrfsConstants:
    library_env: str = "ACTRAIL_CHECKPOINT_BTRFS_LIBRARY"
    library_filename: str = "libactrail_ckpt_fs_btrfs.so"
    backend_instance: str = "project-btrfs"
    filesystem_bytes: int = 1 << 30
    command_timeout_seconds: int = 60
    daemon_startup_wait_ms: int = 90_000
    launch_timeout_seconds: int = 120
    wait_timeout_seconds: float = 30
    poll_seconds: float = 0.1
    stop_timeout_seconds: int = 10
    required_tools: tuple[str, ...] = ("modprobe", "mount", "umount", "mkfs.btrfs", "btrfs", "findmnt", "cp", "curl", "openssl")


CASE_CONSTANTS = CheckpointBtrfsConstants()
