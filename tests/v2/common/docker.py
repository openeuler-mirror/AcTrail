"""Docker daemon reachability shared by container prerequisites and runtimes."""

from __future__ import annotations

import subprocess
from collections.abc import Mapping
from dataclasses import dataclass


@dataclass(frozen=True)
class DockerServerProbe:
    version: str | None
    diagnostic: str


def probe_docker_server(
    environment: Mapping[str, str] | None,
    timeout_seconds: float,
) -> DockerServerProbe:
    """Ask the daemon for its version.

    `docker info --format` exits 0 on some Docker releases (25.x) even when the daemon is
    unreachable, so the daemon counts as ready only when `docker version` exits 0 and prints
    a non-empty server version.
    """
    try:
        completed = subprocess.run(
            ["docker", "version", "--format", "{{.Server.Version}}"],
            env=None if environment is None else dict(environment),
            text=True,
            capture_output=True,
            timeout=timeout_seconds,
            check=False,
        )
    except subprocess.TimeoutExpired:
        return DockerServerProbe(None, f"timed out after {timeout_seconds:g}s")
    except OSError as error:
        return DockerServerProbe(None, f"cannot run Docker probe: {error}")
    version = completed.stdout.strip()
    if completed.returncode == 0 and version:
        return DockerServerProbe(version, "")
    diagnostic = completed.stderr.strip() or completed.stdout.strip()
    if not diagnostic:
        diagnostic = f"exit={completed.returncode} with an empty server version"
    return DockerServerProbe(None, diagnostic)
