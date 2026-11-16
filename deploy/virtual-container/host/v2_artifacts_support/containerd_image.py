"""containerd workload image helpers shared by preparation and the V2 runner."""

from __future__ import annotations

import hashlib
import os
import re
import tempfile
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path


_SHA256 = re.compile(r"^[0-9a-f]{64}$")
_ARCHIVE_COPY_CHUNK_BYTES = 1024 * 1024
_FIRECRACKER_WORKLOAD_PREFIX = (
    "docker.io/library/actrail-firecracker-workload:actrail-"
)


def firecracker_workload_reference(cache_key: str) -> str:
    """Return the one valid derived workload image name for an artifact."""
    if _SHA256.fullmatch(cache_key) is None:
        raise ValueError(
            "Firecracker workload image cache key must be 64 lowercase "
            "hexadecimal characters"
        )
    return _FIRECRACKER_WORKLOAD_PREFIX + cache_key


@contextmanager
def verified_image_archive_snapshot(
    archive: Path,
    expected_sha256: str,
) -> Iterator[Path]:
    """Yield a private snapshot made from the exact bytes that were hashed."""
    if _SHA256.fullmatch(expected_sha256) is None:
        raise ValueError(
            "containerd image archive digest must be 64 lowercase "
            "hexadecimal characters"
        )
    with tempfile.TemporaryDirectory(
        prefix="actrail-image-import.",
    ) as raw_directory:
        snapshot = Path(raw_directory) / "workload-image.tar"
        digest = hashlib.sha256()
        with archive.open("rb") as source, snapshot.open("xb") as destination:
            while block := source.read(_ARCHIVE_COPY_CHUNK_BYTES):
                digest.update(block)
                destination.write(block)
            destination.flush()
            os.fsync(destination.fileno())
        actual_sha256 = digest.hexdigest()
        if actual_sha256 != expected_sha256:
            raise RuntimeError(
                "containerd image archive digest mismatch: "
                f"expected={expected_sha256} actual={actual_sha256}"
            )
        yield snapshot


def containerd_image_check_ready(output: str, reference: str) -> bool:
    """Return true only for a complete row explicitly unpacked by ctr."""
    for line in output.splitlines():
        columns = line.split()
        if len(columns) < 6 or columns[0] != reference:
            continue
        return columns[3] == "complete" and columns[-1].lower() == "true"
    return False
