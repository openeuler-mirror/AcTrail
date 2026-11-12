"""Locked artifact replacement, activation and conservative garbage collection."""

from __future__ import annotations

import errno
import fcntl
import os
import re
import shutil
import stat
import tempfile
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Iterator


_DIGEST = r"[0-9a-f]{64}"
_MANAGED = re.compile(
    rf"(?:{_DIGEST}|\.{_DIGEST}\.(?:backup|build\.[\w-]+|[a-z0-9_]{{8}}))"
)
_VALIDATION_ERRORS = (OSError, RuntimeError, ValueError)


@dataclass(frozen=True)
class ProcessReferences:
    names: frozenset[str] = frozenset()
    uncertain: str | None = None


def process_references(
    root: Path,
    current_root: Path | None,
    proc_root: Path,
) -> ProcessReferences:
    """Find open, mapped, mounted and command-line references; fail closed."""
    names: set[str] = set()
    uncertain: str | None = None
    prefixes = [root]
    if current_root is not None:
        prefixes.append(current_root)
    patterns = [re.compile(re.escape(str(p)) + r"/([^/\s\x00\"']+)") for p in prefixes]

    def collect(value: str) -> None:
        value = value.replace(r"\040", " ")
        names.update(patterns[0].findall(value))
        if current_root is not None:
            for name in patterns[1].findall(value):
                target = (current_root / name).resolve()
                if target.is_relative_to(root) and target != root:
                    names.add(target.relative_to(root).parts[0])

    def unavailable(path: Path, error: OSError) -> None:
        nonlocal uncertain
        if error.errno not in {errno.ENOENT, errno.ESRCH}:
            uncertain = f"cannot inspect {path}: {error}"

    try:
        processes = list(proc_root.iterdir())
    except OSError as error:
        return ProcessReferences(uncertain=f"cannot inspect {proc_root}: {error}")
    for process in processes:
        if not process.name.isdigit() or int(process.name) == os.getpid():
            continue
        for name in ("cmdline", "maps", "mountinfo"):
            path = process / name
            try:
                collect(path.read_bytes().decode("utf-8", errors="surrogateescape"))
            except OSError as error:
                unavailable(path, error)
        try:
            descriptors = list((process / "fd").iterdir())
        except OSError as error:
            unavailable(process / "fd", error)
            descriptors = []
        for path in [*descriptors, process / "cwd", process / "root", process / "exe"]:
            try:
                collect(os.readlink(path))
            except OSError as error:
                unavailable(path, error)
    return ProcessReferences(frozenset(names), uncertain)


class ArtifactStore:
    def __init__(self, root: Path, *, proc_root: Path = Path("/proc")) -> None:
        self.root = root
        self.proc_root = proc_root
        self.current_root: Path | None = None

    @contextmanager
    def locked(self, *, check_only: bool = False) -> Iterator[None]:
        # A check-only invocation must not create directories or a lock file.
        lock = self.root / ".prepare.lock"
        if check_only and not lock.exists():
            yield
            return
        if not check_only:
            self.root.mkdir(parents=True, exist_ok=True)
        flags = os.O_RDONLY | os.O_NOFOLLOW
        if not check_only:
            flags |= os.O_CREAT
        descriptor = os.open(lock, flags, 0o644)
        try:
            if not stat.S_ISREG(os.fstat(descriptor).st_mode):
                raise RuntimeError(f"artifact lock is not a regular file: {lock}")
            fcntl.flock(descriptor, fcntl.LOCK_SH if check_only else fcntl.LOCK_EX)
            yield
        finally:
            os.close(descriptor)

    def references(self) -> ProcessReferences:
        return process_references(self.root, self.current_root, self.proc_root)

    def assert_unused(self, path: Path) -> None:
        references = self.references()
        if references.uncertain is not None or path.name in references.names:
            reason = references.uncertain or "referenced by a running process"
            raise RuntimeError(f"cannot replace in-use artifact {path}: {reason}")

    def backup(self, final: Path) -> Path:
        return self.root / f".{final.name}.backup"

    def recover(self, final: Path, validate: Callable[[Path], None]) -> None:
        backup = self.backup(final)
        if not os.path.lexists(backup):
            return
        if os.path.lexists(final):
            try:
                if final.is_symlink():
                    raise RuntimeError("artifact directory is a symlink")
                validate(final)
            except _VALIDATION_ERRORS:
                self.assert_unused(final)
                self.assert_unused(backup)
                self.remove(final)
            else:
                return  # Valid new publication; remove backup after success.
        else:
            self.assert_unused(backup)
        os.replace(backup, final)
        self.sync()
        print(f"artifact_recovery=restored path={final}", flush=True)

    def publish(
        self,
        staging: Path,
        final: Path,
        validate: Callable[[Path], None],
    ) -> Path:
        backup = self.backup(final)
        if os.path.lexists(backup):
            raise RuntimeError(f"unrecovered artifact backup exists: {backup}")
        had_previous = os.path.lexists(final)
        moved_previous = False
        published = False
        try:
            if had_previous:
                self.assert_unused(final)
                os.replace(final, backup)
                moved_previous = True
                self.sync()
            os.replace(staging, final)
            published = True
            self.sync()
            validate(final)
        except BaseException:
            if published:
                self.remove(final)
            if moved_previous:
                os.replace(backup, final)
            self.sync()
            raise
        return final / "manifest.json"

    def activate(self, manifest: Path, *, backend: str, egress_mode: str) -> None:
        current = self.current_root
        if current is None:
            return
        if current == self.root or current.is_relative_to(self.root):
            raise ValueError("current root must be outside the artifact root")
        current.mkdir(parents=True, exist_ok=True)
        mode = "vsock" if egress_mode == "vsock-bridge" else "network"
        link = current / f"{backend}-{mode}"
        descriptor, name = tempfile.mkstemp(prefix=f".{link.name}.", dir=current)
        os.close(descriptor)
        temporary = Path(name)
        try:
            temporary.unlink()
            temporary.symlink_to(os.path.relpath(manifest.parent, current))
            os.replace(temporary, link)
            self.sync(current)
        finally:
            temporary.unlink(missing_ok=True)
        print(f"artifact_current={link}", flush=True)

    def cleanup(self, manifest: Path, *, inputs: list[Path]) -> None:
        # Source/test callers only remove their own completed replacement backup.
        candidates = (
            list(self.root.iterdir())
            if self.current_root is not None
            else [self.backup(manifest.parent)]
        )
        candidates = [
            p for p in candidates if _MANAGED.fullmatch(p.name) and os.path.lexists(p)
        ]
        preserved = {manifest.parent.name}
        for path in inputs:
            path = path.resolve()
            if path.is_relative_to(self.root) and path != self.root:
                preserved.add(path.relative_to(self.root).parts[0])
        if self.current_root is not None:
            for link in self.current_root.iterdir():
                if link.name.startswith("."):
                    continue
                if not link.is_symlink():
                    print(f"artifact_cleanup=skipped reason=unknown-current-entry path={link}", flush=True)
                    return
                target = link.resolve()
                if target.is_relative_to(self.root) and target != self.root:
                    preserved.add(target.relative_to(self.root).parts[0])
        if not candidates:
            return
        references = self.references()
        if references.uncertain is not None:
            print(f"artifact_cleanup=skipped reason={references.uncertain}", flush=True)
            return
        preserved.update(references.names)
        for path in sorted(candidates):
            recovering_current = (
                path.name.endswith(".backup")
                and path.name[1:65] in preserved
                and not os.path.lexists(self.root / path.name[1:65])
            )
            if path.name in preserved or recovering_current or (
                path.is_symlink() and not path.name.endswith(".backup")
            ):
                print(f"artifact_cleanup=kept path={path}", flush=True)
                continue
            try:
                self.remove(path)
                self.sync()
            except OSError as error:
                raise RuntimeError(f"artifact cleanup failed for {path}: {error}") from error
            print(f"artifact_cleanup=removed path={path}", flush=True)

    def remove(self, path: Path) -> None:
        if path.parent != self.root or not _MANAGED.fullmatch(path.name):
            raise ValueError(f"refusing to remove unmanaged artifact path: {path}")
        if path.is_dir() and not path.is_symlink():
            shutil.rmtree(path)
        else:
            path.unlink()

    def sync(self, directory: Path | None = None) -> None:
        descriptor = os.open(directory or self.root, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
