"""Resolve user-supplied command names or launchers to concrete ELF files."""

from __future__ import annotations

import shlex
import shutil
from pathlib import Path


ELF_MAGIC = b"\x7fELF"
KNOWN_TLS_LAUNCHER_RUNTIMES = ("node", "bun")


def resolve_entry_elf(path: Path) -> Path:
    entry = resolve_command_or_path(path)
    if is_elf(entry):
        return entry
    candidate = non_elf_entry_candidate(entry)
    if candidate is None:
        raise RuntimeError(f"{entry} is not an ELF file and no concrete ELF runtime was found")
    return candidate


def resolve_command_or_path(path: Path) -> Path:
    raw = str(path)
    if "/" not in raw:
        resolved = shutil.which(raw)
        if resolved is None:
            raise RuntimeError(f"command not found on PATH: {raw}")
        return Path(resolved).resolve()
    resolved = path.resolve()
    if not resolved.is_file():
        raise RuntimeError(f"not a file: {resolved}")
    return resolved


def is_elf(path: Path) -> bool:
    try:
        with path.open("rb") as handle:
            return handle.read(len(ELF_MAGIC)) == ELF_MAGIC
    except OSError:
        return False


def non_elf_entry_candidate(entry: Path) -> Path | None:
    sibling = entry.parent / f".{entry.name}"
    if sibling.is_file() and is_elf(sibling):
        return sibling.resolve()
    return resolve_shebang_runtime(entry)


def resolve_shebang_runtime(entry: Path) -> Path | None:
    try:
        first_line = entry.read_text(encoding="utf-8", errors="ignore").splitlines()[0]
    except (IndexError, OSError):
        return None
    if not first_line.startswith("#!"):
        return None
    parts = shlex.split(first_line[2:].strip())
    if not parts:
        return None
    runtime = shebang_runtime_name(parts)
    if runtime not in KNOWN_TLS_LAUNCHER_RUNTIMES:
        return None
    resolved = shutil.which(runtime)
    if resolved is None:
        raise RuntimeError(f"{entry} uses {runtime} in shebang, but {runtime} is not on PATH")
    binary = Path(resolved).resolve()
    if not is_elf(binary):
        raise RuntimeError(f"{entry} shebang runtime {binary} is not an ELF file")
    return binary


def shebang_runtime_name(parts: list[str]) -> str:
    command = Path(parts[0]).name
    if command != "env":
        return command
    if len(parts) > 1 and parts[1] == "-S":
        if len(parts) < 3:
            raise RuntimeError("env -S shebang is missing command")
        split = shlex.split(" ".join(parts[2:]))
        if not split:
            raise RuntimeError("env -S shebang is missing command")
        return split[0]
    for part in parts[1:]:
        if not part.startswith("-"):
            return part
    return ""
