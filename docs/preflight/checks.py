"""Read-only platform, toolchain, and agent TLS checks."""

from __future__ import annotations

import os
import platform
import re
import shutil
from dataclasses import dataclass
from pathlib import Path

from .common import FAIL, PASS, WARN, Check, last_line, read_text, run_command


SUPPORTED_ARCHITECTURES = {"x86_64", "aarch64"}
RELEASE_BINARIES = ("actraild", "actrailctl", "actrailviewer", "ebpf_probe")
TLS_SYNC_LIBRARY = "libactrail_tls_payload_probe_sync.so"
RELEASE_ARTIFACTS = RELEASE_BINARIES + (TLS_SYNC_LIBRARY,)
OPENSSL_REQUIRED_SYMBOLS = ("SSL_read", "SSL_write", "SSL_read_ex", "SSL_write_ex")
OPENSSL_OPTIONAL_SYMBOLS = ("SSL_write_ex2",)
OPENSSL_SYMBOLS = OPENSSL_REQUIRED_SYMBOLS + OPENSSL_OPTIONAL_SYMBOLS


@dataclass(frozen=True)
class ResolvedArtifact:
    name: str
    path: Path | None
    detail: str


@dataclass(frozen=True)
class ResolvedArtifacts:
    spec: Path
    values: dict[str, ResolvedArtifact]

    def path(self, name: str) -> Path | None:
        return self.values[name].path

    def detail(self, name: str) -> str:
        return self.values[name].detail


def resolve_release_artifacts(bin_spec: str | Path) -> ResolvedArtifacts:
    spec = normalize_bin_spec(bin_spec)
    bad_file = spec.exists() and spec.is_file() and spec.name not in RELEASE_ARTIFACTS
    direct_artifact = spec.name if spec.name in RELEASE_ARTIFACTS else None
    candidate_dir = spec.parent if direct_artifact else spec
    values: dict[str, ResolvedArtifact] = {}
    for name in RELEASE_ARTIFACTS:
        if bad_file:
            values[name] = ResolvedArtifact(
                name=name,
                path=None,
                detail=(
                    f"{spec} is a file, but --bin-dir/ACTRAIL_BIN_DIR must be a directory "
                    f"or one of: {', '.join(RELEASE_ARTIFACTS)}"
                ),
            )
            continue
        candidate = spec if direct_artifact == name else candidate_dir / name
        source = "configured artifact" if direct_artifact == name else "configured directory"
        if direct_artifact and direct_artifact != name:
            source = f"sibling of configured {direct_artifact}"
        values[name] = resolve_release_artifact(name, candidate, source)
    return ResolvedArtifacts(spec=spec, values=values)


def normalize_bin_spec(bin_spec: str | Path) -> Path:
    path = Path(bin_spec).expanduser()
    if not path.is_absolute():
        path = Path.cwd() / path
    return path


def resolve_release_artifact(name: str, candidate: Path, source: str) -> ResolvedArtifact:
    if name == TLS_SYNC_LIBRARY:
        return resolve_tls_sync_library(candidate, source)
    if candidate.exists():
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return ResolvedArtifact(name=name, path=candidate.resolve(), detail=f"{candidate.resolve()} ({source})")
        return ResolvedArtifact(
            name=name,
            path=None,
            detail=f"{candidate} exists but is not an executable file",
        )
    path = shutil.which(name)
    if path:
        resolved = Path(path).resolve()
        return ResolvedArtifact(name=name, path=resolved, detail=f"{resolved} (PATH; missing {candidate})")
    return ResolvedArtifact(
        name=name,
        path=None,
        detail=(
            f"missing {candidate} and not found on PATH; run cargo build --release "
            "or set ACTRAIL_BIN_DIR to a release directory or release binary path"
        ),
    )


def resolve_tls_sync_library(candidate: Path, source: str) -> ResolvedArtifact:
    env_path = os.environ.get("TLS_PAYLOAD_SYNC_LIBRARY")
    if env_path:
        path = normalize_bin_spec(env_path)
        if path.is_file() and os.access(path, os.R_OK):
            return ResolvedArtifact(
                name=TLS_SYNC_LIBRARY,
                path=path.resolve(),
                detail=f"{path.resolve()} (TLS_PAYLOAD_SYNC_LIBRARY)",
            )
        return ResolvedArtifact(
            name=TLS_SYNC_LIBRARY,
            path=None,
            detail=f"TLS_PAYLOAD_SYNC_LIBRARY={path} is not a readable file",
        )
    if candidate.exists():
        if candidate.is_file() and os.access(candidate, os.R_OK):
            return ResolvedArtifact(
                name=TLS_SYNC_LIBRARY,
                path=candidate.resolve(),
                detail=f"{candidate.resolve()} ({source})",
            )
        return ResolvedArtifact(
            name=TLS_SYNC_LIBRARY,
            path=None,
            detail=f"{candidate} exists but is not a readable file",
        )
    return ResolvedArtifact(
        name=TLS_SYNC_LIBRARY,
        path=None,
        detail=(
            f"missing {candidate}; build with cargo build --release -p tls_payload_probe_sync "
            "or set TLS_PAYLOAD_SYNC_LIBRARY"
        ),
    )


def platform_checks() -> list[Check]:
    machine = platform.machine()
    arch_status = PASS if machine in SUPPORTED_ARCHITECTURES else FAIL
    checks = [
        Check(
            "architecture",
            arch_status,
            f"{machine}; supported={', '.join(sorted(SUPPORTED_ARCHITECTURES))}",
        ),
        Check(
            "effective uid",
            PASS if os.geteuid() == 0 else FAIL,
            f"uid={os.geteuid()}; live collection requires root or equivalent capabilities",
        ),
    ]
    os_release = read_os_release()
    if os_release:
        checks.append(Check("linux distribution", PASS, os_release, required=False))
    checks.append(Check("kernel release", PASS, platform.release(), required=False))
    if "microsoft" in read_text(Path("/proc/version")).lower():
        checks.append(Check("WSL detection", PASS, "/proc/version contains Microsoft", False))
    return checks


def read_os_release() -> str:
    values: dict[str, str] = {}
    path = Path("/etc/os-release")
    if not path.exists():
        return ""
    for raw in path.read_text(encoding="utf-8", errors="ignore").splitlines():
        key, separator, value = raw.partition("=")
        if separator:
            values[key] = value.strip().strip('"')
    return values.get("PRETTY_NAME") or " ".join(
        value for value in (values.get("ID"), values.get("VERSION_ID")) if value
    )


def release_artifact_checks(artifacts: ResolvedArtifacts) -> list[Check]:
    checks: list[Check] = []
    for name in RELEASE_ARTIFACTS:
        artifact = artifacts.values[name]
        checks.append(
            Check(
                name,
                PASS if artifact.path is not None else FAIL,
                artifact.detail,
            )
        )
    return checks


def kernel_checks() -> list[Check]:
    checks = [
        Check(
            "kernel BTF",
            PASS if os.access("/sys/kernel/btf/vmlinux", os.R_OK) else FAIL,
            "/sys/kernel/btf/vmlinux readable"
            if os.access("/sys/kernel/btf/vmlinux", os.R_OK)
            else "/sys/kernel/btf/vmlinux is missing or unreadable",
        )
    ]
    checks.extend(tracefs_checks())
    checks.extend(sysctl_checks())
    checks.extend(seccomp_checks())
    return checks


def tracefs_checks() -> list[Check]:
    mounts = tracefs_mounts()
    if not mounts:
        return [Check("tracefs mount", FAIL, "tracefs was not found in /proc/self/mountinfo")]
    writable = [mount for mount in mounts if os.access(mount, os.W_OK)]
    if writable:
        return [Check("tracefs writable mount", PASS, ", ".join(str(path) for path in writable))]
    return [
        Check(
            "tracefs writable mount",
            FAIL,
            "tracefs mounted but none are writable: " + ", ".join(str(path) for path in mounts),
        )
    ]


def tracefs_mounts() -> list[Path]:
    mounts: list[Path] = []
    for raw in read_text(Path("/proc/self/mountinfo")).splitlines():
        left, separator, right = raw.partition(" - ")
        if not separator:
            continue
        fields = left.split()
        right_fields = right.split()
        if len(fields) > 4 and right_fields and right_fields[0] == "tracefs":
            mounts.append(Path(fields[4]))
    for fallback in (Path("/sys/kernel/tracing"), Path("/sys/kernel/debug/tracing")):
        if fallback.exists() and fallback not in mounts:
            mounts.append(fallback)
    return mounts


def sysctl_checks() -> list[Check]:
    return [
        sysctl_check("kernel.perf_event_paranoid", Path("/proc/sys/kernel/perf_event_paranoid")),
        sysctl_check(
            "kernel.unprivileged_bpf_disabled",
            Path("/proc/sys/kernel/unprivileged_bpf_disabled"),
            required=False,
        ),
    ]


def sysctl_check(name: str, path: Path, required: bool = True) -> Check:
    raw = read_text(path).strip()
    if not raw:
        return Check(name, WARN if not required else FAIL, f"{path} is unreadable", required)
    if name == "kernel.perf_event_paranoid":
        try:
            value = int(raw)
        except ValueError:
            return Check(name, WARN, f"value={raw}; cannot interpret", required)
        if value > 2:
            return Check(name, WARN, f"value={value}; eBPF attach may be blocked", required)
    return Check(name, PASS, f"value={raw}", required)


def seccomp_checks() -> list[Check]:
    actions = read_text(Path("/proc/sys/kernel/seccomp/actions_avail")).split()
    if not actions:
        return [Check("seccomp actions", FAIL, "actions_avail is unreadable")]
    user_notif = "user_notif" in actions
    return [
        Check("seccomp user notification", PASS if user_notif else FAIL, " ".join(actions)),
    ]


def tool_checks() -> list[Check]:
    checks = [command_check("python3", True)]
    for name in ("cargo", "clang", "nm", "readelf", "ldconfig", "openssl"):
        checks.append(command_check(name, True))
    checks.extend(curl_checks())
    return checks


def command_check(name: str, required: bool) -> Check:
    path = shutil.which(name)
    return Check(name, PASS if path else FAIL, path or "not found on PATH", required)


def curl_checks() -> list[Check]:
    curl = shutil.which("curl")
    if not curl:
        return [Check("curl", FAIL, "not found on PATH")]
    result = run_command((curl, "--version"))
    if result.returncode != 0:
        return [Check("curl", FAIL, last_line(result.stderr) or "curl --version failed")]
    output = result.stdout
    first = output.splitlines()[0] if output.splitlines() else "curl --version produced no output"
    checks = [Check("curl", PASS, first)]
    tls_ok = "OpenSSL" in first or "LibreSSL" in first
    checks.append(
        Check(
            "curl OpenSSL-compatible TLS",
            PASS if tls_ok else FAIL,
            first if tls_ok else f"{first}; local HTTPS payload examples expect libssl symbols",
        )
    )
    checks.append(
        Check(
            "curl HTTP/2 feature",
            PASS if "HTTP2" in output else FAIL,
            "HTTP2 feature present" if "HTTP2" in output else "HTTP2 feature missing",
        )
    )
    return checks


def shared_openssl_checks() -> list[Check]:
    ldconfig = shutil.which("ldconfig")
    if not ldconfig:
        return [Check("libssl lookup", FAIL, "ldconfig is not on PATH")]
    result = run_command((ldconfig, "-p"))
    if result.returncode != 0:
        return [Check("libssl lookup", FAIL, last_line(result.stderr) or "ldconfig -p failed")]
    libssl = find_libssl(result.stdout)
    if libssl is None:
        return [Check("libssl shared object", FAIL, "ldconfig did not report libssl.so")]
    return [
        Check("libssl shared object", PASS, str(libssl)),
        symbol_check("libssl symbols", libssl, required=True),
    ]


def find_libssl(output: str) -> Path | None:
    for line in output.splitlines():
        if "libssl.so" not in line or "=>" not in line:
            continue
        candidate = Path(line.rsplit("=>", 1)[1].strip())
        if candidate.exists():
            return candidate
    return None


def agent_tls_checks() -> list[Check]:
    checks: list[Check] = []
    checks.extend(executable_tls_checks("claude", expected_runtime=None))
    checks.extend(executable_tls_checks("opencode", expected_runtime=None))
    return checks


def executable_tls_checks(command: str, expected_runtime: str | None) -> list[Check]:
    path = shutil.which(command)
    if not path:
        return [Check(command, FAIL, "not found on PATH", required=False)]
    entry = Path(path).resolve()
    checks = [Check(command, PASS, str(entry), required=False)]
    runtime = resolve_runtime(entry)
    if runtime is None:
        checks.append(check_entry_symbols(command, entry))
        return checks
    runtime_status = FAIL if str(runtime.path).startswith("<") else PASS
    checks.append(Check(f"{command} runtime", runtime_status, f"{runtime.name} -> {runtime.path}", False))
    if expected_runtime and runtime.name != expected_runtime:
        checks.append(
            Check(f"{command} expected runtime", FAIL, f"expected {expected_runtime}, got {runtime.name}", False)
        )
    checks.extend(runtime_symbol_checks(command, runtime))
    return checks


def check_entry_symbols(command: str, entry: Path) -> Check:
    if is_elf(entry):
        check = symbol_check(f"{command} TLS symbols", entry, required=False)
        if command == "claude" and check.status == FAIL:
            return Check(
                check.name,
                WARN,
                check.detail + "; E2E will also try configured static-BoringSSL discovery",
                required=False,
            )
        return check
    return Check(f"{command} runtime", FAIL, "entrypoint is not ELF and has no known runtime", False)


def runtime_symbol_checks(command: str, runtime: "Runtime") -> list[Check]:
    if runtime.name == "node":
        return [symbol_check(f"{command} Node OpenSSL symbols", runtime.path, required=False)]
    if runtime.name == "bun":
        return [
            Check(
                f"{command} Bun TLS symbols",
                WARN,
                "Bun often uses static/BoringSSL TLS; dynamic OpenSSL symbol scan is not decisive",
                required=False,
            ),
            symbol_check(f"{command} Bun dynamic symbols", runtime.path, required=False),
        ]
    return [symbol_check(f"{command} runtime TLS symbols", runtime.path, required=False)]


@dataclass(frozen=True)
class Runtime:
    name: str
    path: Path


def resolve_runtime(entry: Path) -> Runtime | None:
    first = first_line(entry)
    for name in ("node", "bun"):
        if name in first:
            path = shutil.which(name)
            if path:
                return Runtime(name=name, path=Path(path).resolve())
            return Runtime(name=name, path=Path(f"<{name}-not-found>"))
    return None


def first_line(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8", errors="ignore").splitlines()[0]
    except (IndexError, OSError, UnicodeDecodeError):
        return ""


def is_elf(path: Path) -> bool:
    try:
        return path.read_bytes().startswith(b"\x7fELF")
    except OSError:
        return False


def symbol_check(name: str, binary: Path, required: bool) -> Check:
    if str(binary).startswith("<"):
        return Check(name, FAIL, f"{binary} is unavailable", required)
    readelf = shutil.which("readelf")
    if not readelf:
        return Check(name, FAIL, "readelf is not on PATH", required)
    result = run_command((readelf, "-Ws", str(binary)))
    if result.returncode != 0:
        return Check(name, FAIL, last_line(result.stderr) or f"readelf -Ws failed for {binary}", required)
    symbols = elf_function_symbols(result.stdout)
    missing = [symbol for symbol in OPENSSL_REQUIRED_SYMBOLS if symbol not in symbols]
    if missing:
        return Check(name, FAIL, f"missing {', '.join(missing)} in {binary}", required)
    optional_present = [symbol for symbol in OPENSSL_OPTIONAL_SYMBOLS if symbol in symbols]
    optional_detail = (
        f"; optional present: {', '.join(optional_present)}"
        if optional_present
        else f"; optional not exported: {', '.join(OPENSSL_OPTIONAL_SYMBOLS)}"
    )
    return Check(
        name,
        PASS,
        f"{binary} exports required {', '.join(OPENSSL_REQUIRED_SYMBOLS)}{optional_detail}",
        required,
    )


def elf_function_symbols(output: str) -> set[str]:
    symbols: set[str] = set()
    for line in output.splitlines():
        parts = line.split()
        if len(parts) < 8 or not parts[0].endswith(":"):
            continue
        if parts[3] == "FUNC" and parts[6] != "UND":
            symbols.add(parts[7].split("@", 1)[0])
    return symbols
