from __future__ import annotations

import base64
import io
import re
import tarfile
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol

from tests.v2.common.kata_runtime import KataTestContainer


_SAFE_COORDINATION_NAME = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")
_GUEST_ASSET_ROOT = Path("/opt/actrail-execution")
_GUEST_COORDINATION_ROOT = Path("/run/actrail-execution")


class CoordinationFile(Protocol):
    def is_file(self) -> bool: ...

    def touch(self) -> None: ...

    def read_text(self, *, encoding: str) -> str: ...


class CoordinationDirectory(Protocol):
    def file(self, name: str) -> CoordinationFile: ...


@dataclass(frozen=True)
class HostCoordination:
    root: Path

    def file(self, name: str) -> Path:
        return self.root / _coordination_name(name)


@dataclass(frozen=True)
class GuestCoordination:
    vm: KataTestContainer
    uid: int
    gid: int
    timeout_seconds: float
    root: Path = _GUEST_COORDINATION_ROOT

    def file(self, name: str) -> GuestCoordinationFile:
        return GuestCoordinationFile(
            self.vm,
            self.root / _coordination_name(name),
            self.uid,
            self.gid,
            self.timeout_seconds,
        )


@dataclass(frozen=True)
class GuestCoordinationFile:
    vm: KataTestContainer
    path: Path
    uid: int
    gid: int
    timeout_seconds: float

    def is_file(self) -> bool:
        result = self.vm.exec(
            ("/bin/sh", "-c", 'test -f "$1"', "actrail-coord", str(self.path)),
            uid=self.uid,
            gid=self.gid,
            timeout=self.timeout_seconds,
        )
        if result.returncode == 0:
            return True
        if result.returncode == 1:
            return False
        raise RuntimeError(
            f"cannot inspect Guest coordination file {self.path}: "
            f"{result.diagnostic or f'exit={result.returncode}'}"
        )

    def touch(self) -> None:
        result = self.vm.exec(
            (
                "/bin/sh",
                "-c",
                'umask 007; touch -- "$1"',
                "actrail-coord",
                str(self.path),
            ),
            uid=self.uid,
            gid=self.gid,
            timeout=self.timeout_seconds,
        )
        if result.returncode != 0:
            raise RuntimeError(
                f"cannot create Guest coordination file {self.path}: "
                f"{result.diagnostic or f'exit={result.returncode}'}"
            )

    def read_text(self, *, encoding: str) -> str:
        if encoding not in {"ascii", "utf-8"}:
            raise ValueError(f"unsupported Guest coordination encoding: {encoding}")
        result = self.vm.exec(
            (
                "/bin/sh",
                "-c",
                'test -f "$1" && cat -- "$1"',
                "actrail-coord",
                str(self.path),
            ),
            uid=self.uid,
            gid=self.gid,
            timeout=self.timeout_seconds,
        )
        if result.returncode != 0:
            raise RuntimeError(
                f"cannot read Guest coordination file {self.path}: "
                f"{result.diagnostic or f'exit={result.returncode}'}"
            )
        return result.stdout.encode("utf-8").decode(encoding)


@dataclass(frozen=True)
class FirecrackerAssetTransport:
    assets: Path
    uid: int
    gid: int
    timeout_seconds: float
    asset_root: Path = _GUEST_ASSET_ROOT
    coordination_root: Path = _GUEST_COORDINATION_ROOT

    def stage(self, vm: KataTestContainer) -> None:
        encoded_archive = _encoded_asset_archive(
            self.assets,
            excluded_names=frozenset({"xiaoo-real"}),
        )
        result = vm.exec(
            (
                "python3",
                "-c",
                _GUEST_STAGE_SCRIPT,
                str(self.asset_root),
                str(self.coordination_root),
                str(self.uid),
                str(self.gid),
                str(len(encoded_archive)),
            ),
            uid=0,
            gid=0,
            timeout=self.timeout_seconds,
            input_text=encoded_archive,
        )
        if result.returncode != 0:
            raise RuntimeError(
                "failed to stage execution-isolation assets into the Firecracker "
                "workload container: "
                + (result.diagnostic or f"exit={result.returncode}")
            )
        verification = vm.exec(
            (
                "/bin/sh",
                "-ec",
                'cd "$1"; sha256sum --check MANIFEST.sha256; '
                "test -x xiaoo-real; test -x xiaoo-root; "
                "./xiaoo-real --cli run --help 2>&1 | grep -q -- --tools",
                "actrail-stage",
                str(self.asset_root),
            ),
            uid=0,
            gid=0,
            timeout=self.timeout_seconds,
        )
        if verification.returncode != 0:
            raise RuntimeError(
                "staged Firecracker workload assets failed verification: "
                + (verification.diagnostic or f"exit={verification.returncode}")
            )


def _coordination_name(name: str) -> str:
    if not _SAFE_COORDINATION_NAME.fullmatch(name):
        raise ValueError(f"unsafe coordination file name: {name!r}")
    return name


def _encoded_asset_archive(
    directory: Path,
    *,
    excluded_names: frozenset[str] = frozenset(),
) -> str:
    if not directory.is_dir():
        raise RuntimeError(
            f"execution-isolation asset directory is missing: {directory}"
        )
    buffer = io.BytesIO()
    names: set[str] = set()
    excluded_found: set[str] = set()
    with tarfile.open(
        fileobj=buffer,
        mode="w:gz",
        format=tarfile.PAX_FORMAT,
    ) as archive:
        for path in sorted(directory.iterdir()):
            if path.is_symlink() or not path.is_file():
                raise RuntimeError(
                    f"execution-isolation asset must be a regular file: {path}"
                )
            if path.name in names:
                raise RuntimeError(f"duplicate execution-isolation asset: {path.name}")
            if path.name in excluded_names:
                excluded_found.add(path.name)
                continue
            names.add(path.name)
            info = archive.gettarinfo(str(path), arcname=path.name)
            info.uid = 0
            info.gid = 0
            info.uname = ""
            info.gname = ""
            info.mode = 0o755 if path.stat().st_mode & 0o111 else 0o644
            with path.open("rb") as source:
                archive.addfile(info, source)
    if not names or "MANIFEST.sha256" not in names:
        raise RuntimeError("execution-isolation assets require MANIFEST.sha256")
    missing_exclusions = excluded_names - excluded_found
    if missing_exclusions:
        raise RuntimeError(
            "execution-isolation preinstalled asset is missing: "
            + ", ".join(sorted(missing_exclusions))
        )
    return base64.b64encode(buffer.getvalue()).decode("ascii")


_GUEST_STAGE_SCRIPT = r"""
import base64
import io
import os
import pathlib
import sys
import tarfile
import tempfile

asset_root = pathlib.Path(sys.argv[1])
coordination_root = pathlib.Path(sys.argv[2])
workload_uid = int(sys.argv[3])
workload_gid = int(sys.argv[4])
encoded_size = int(sys.argv[5])
if not 1 <= encoded_size <= 64 * 1024 * 1024:
    raise RuntimeError(f"invalid encoded asset archive size: {encoded_size}")
encoded_payload = sys.stdin.buffer.read(encoded_size)
if len(encoded_payload) != encoded_size:
    raise RuntimeError(
        "truncated encoded asset archive: "
        f"expected={encoded_size} actual={len(encoded_payload)}"
    )
payload = base64.b64decode(encoded_payload, validate=True)

asset_root.mkdir(parents=True, exist_ok=True)
if asset_root.is_symlink() or not asset_root.is_dir():
    raise RuntimeError(f"unsafe asset root: {asset_root}")
os.chmod(asset_root, 0o755)
coordination_root.mkdir(parents=True, exist_ok=True)
if coordination_root.is_symlink() or not coordination_root.is_dir():
    raise RuntimeError(f"unsafe coordination root: {coordination_root}")
os.chown(coordination_root, workload_uid, workload_gid)
os.chmod(coordination_root, 0o770)

seen = set()
with tarfile.open(fileobj=io.BytesIO(payload), mode="r:gz") as archive:
    members = archive.getmembers()
    if not members:
        raise RuntimeError("asset archive is empty")
    for member in members:
        pure_name = pathlib.PurePosixPath(member.name)
        if (
            not member.isfile()
            or pure_name.name != member.name
            or member.name in {"", ".", ".."}
            or member.name in seen
        ):
            raise RuntimeError(f"unsafe asset archive member: {member.name!r}")
        seen.add(member.name)
        source = archive.extractfile(member)
        if source is None:
            raise RuntimeError(f"asset archive member has no data: {member.name}")
        destination = asset_root / member.name
        if destination.is_symlink():
            raise RuntimeError(f"asset destination is a symlink: {destination}")
        descriptor, temporary_name = tempfile.mkstemp(
            prefix=f".{member.name}.", dir=asset_root
        )
        try:
            with os.fdopen(descriptor, "wb") as temporary:
                while True:
                    block = source.read(1024 * 1024)
                    if not block:
                        break
                    temporary.write(block)
                temporary.flush()
                os.fsync(temporary.fileno())
                os.fchmod(temporary.fileno(), 0o755 if member.mode & 0o111 else 0o644)
            os.replace(temporary_name, destination)
        except BaseException:
            try:
                os.unlink(temporary_name)
            except FileNotFoundError:
                pass
            raise

if "MANIFEST.sha256" not in seen:
    raise RuntimeError("asset archive omitted MANIFEST.sha256")
print(f"ACTRAIL_FIRECRACKER_ASSETS_STAGED files={len(seen)}")
"""
