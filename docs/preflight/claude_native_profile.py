#!/usr/bin/env python3
"""Profile a native Claude Code TLS runtime without exporting the binary."""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
import platform
import shutil
import stat
import subprocess
import sys
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[2]
TLS_RUNTIME_DIR = REPO_ROOT / "tests/agent-trace/runtime_tls"
if str(TLS_RUNTIME_DIR) not in sys.path:
    sys.path.insert(0, str(TLS_RUNTIME_DIR))

from boringssl import binary_build_id, prepare_bun_static_boringssl_map  # noqa: E402


def main() -> int:
    args = parse_args()
    workload = read_key_values(args.workload_config)
    if args.symbol_map_output:
        workload["generated_symbol_map_path"] = str(args.symbol_map_output)
    binary = resolve_claude_binary(args.claude_bin)
    profile = build_profile(binary, workload)
    write_outputs(profile, args)
    print_profile(profile)
    return 0 if profile["status"] == "supported" else 2


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Inspect the local Claude Code executable and generate a TLS symbol map when supported."
    )
    parser.add_argument("--claude-bin", type=Path, help="resolved native Claude Code executable")
    parser.add_argument(
        "--workload-config",
        type=Path,
        default=REPO_ROOT / "tests/payload/claude-code/workload.conf",
        help="Claude payload workload config containing BoringSSL profile settings",
    )
    parser.add_argument(
        "--symbol-map-output",
        type=Path,
        help="where to write a generated bun-static-boringssl symbol map",
    )
    parser.add_argument("--json-output", type=Path, help="where to write the profile JSON")
    return parser.parse_args()


def resolve_claude_binary(explicit: Path | None) -> Path:
    if explicit is not None:
        return require_executable(explicit)
    claude = shutil.which("claude")
    if claude is None:
        raise RuntimeError("claude is not on PATH")
    return require_executable(Path(claude))


def require_executable(path: Path) -> Path:
    resolved = path.resolve()
    try:
        mode = resolved.stat().st_mode
    except OSError as error:
        raise RuntimeError(f"{path} is not readable") from error
    if not stat.S_ISREG(mode) or not os.access(resolved, os.X_OK):
        raise RuntimeError(f"{path} is not an executable file")
    return resolved


def build_profile(binary: Path, workload: dict[str, str]) -> dict[str, Any]:
    build_id = binary_build_id(binary)
    profile: dict[str, Any] = {
        "status": "unsupported",
        "binary": str(binary),
        "arch": platform.machine(),
        "build_id": build_id,
        "sha256": sha256_file(binary),
        "file": file_description(binary),
        "claude_package": claude_package_info(binary),
    }
    openssl_symbols = exported_openssl_symbols(binary)
    profile["openssl_symbols"] = openssl_symbols
    if all(openssl_symbols.values()):
        profile.update(
            {
                "status": "supported",
                "resolver": "openssl-symbols",
                "library": "openssl",
                "symbol_map_path": "disabled",
            }
        )
        return profile
    if platform.machine() in {"aarch64", "x86_64"}:
        profile.update(
            {
                "status": "supported",
                "resolver": "boringssl-static",
                "library": "boringssl",
                "symbol_map_path": "disabled",
                "symbol_map_detail": "built-in static BoringSSL related-entry detector",
            }
        )
        return profile

    configured_map = resolve_path(required(workload, "symbol_map_path"))
    try:
        symbol_map, detail = prepare_bun_static_boringssl_map(binary, configured_map, workload)
    except Exception as error:
        profile.update(
            {
                "status": "profile_missing",
                "resolver": "bun-static-boringssl",
                "library": "boringssl",
                "reason": str(error),
                "next_step": (
                    "Run this profiler on the target host after adding an arch/build-id matching "
                    "BoringSSL profile, or collect this JSON/log for profile generation. The binary "
                    "does not need to leave the target host."
                ),
            }
        )
        return profile

    profile.update(
        {
            "status": "supported",
            "resolver": "bun-static-boringssl",
            "library": "boringssl",
            "symbol_map_path": str(symbol_map),
            "symbol_map_detail": detail,
            "symbol_map_text": symbol_map.read_text(encoding="utf-8"),
        }
    )
    return profile


def exported_openssl_symbols(binary: Path) -> dict[str, bool]:
    output = run_checked(["readelf", "-Ws", str(binary)])
    symbols = set()
    for line in output.splitlines():
        parts = line.split()
        if len(parts) < 8 or not parts[0].endswith(":"):
            continue
        if parts[3] == "FUNC" and parts[6] != "UND":
            symbols.add(parts[7].split("@", 1)[0])
    return {symbol: symbol in symbols for symbol in ("SSL_read", "SSL_write", "SSL_read_ex", "SSL_write_ex")}


def claude_package_info(binary: Path) -> dict[str, Any]:
    package_dir = find_package_dir(binary)
    if package_dir is None:
        return {}
    package = read_json(package_dir / "package.json")
    native_packages = []
    optional = package.get("optionalDependencies", {})
    node_modules = package_dir / "node_modules" / "@anthropic-ai"
    for name, version in sorted(optional.items()):
        if not name.startswith("@anthropic-ai/claude-code-"):
            continue
        installed = node_modules / name.split("/", 1)[1]
        native_packages.append(
            {
                "name": name,
                "version": version,
                "installed": installed.exists(),
                "path": str(installed) if installed.exists() else "",
            }
        )
    return {
        "path": str(package_dir),
        "name": package.get("name", ""),
        "version": package.get("version", ""),
        "bin": package.get("bin", {}),
        "native_packages": native_packages,
    }


def find_package_dir(binary: Path) -> Path | None:
    for directory in binary.parents:
        package_path = directory / "package.json"
        if not package_path.exists():
            continue
        package = read_json(package_path)
        if package.get("name") == "@anthropic-ai/claude-code":
            return directory
    return None


def read_key_values(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        key, separator, value = line.partition("=")
        if not separator:
            raise RuntimeError(f"invalid config line in {path}: {raw}")
        values[key.strip()] = value.strip()
    return values


def read_json(path: Path) -> dict[str, Any]:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return {}


def resolve_path(raw: str) -> Path:
    path = Path(raw)
    return path if path.is_absolute() else REPO_ROOT / path


def required(values: dict[str, str], key: str) -> str:
    value = values.get(key)
    if not value:
        raise RuntimeError(f"missing workload config key {key}")
    return value


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while chunk := handle.read(io.DEFAULT_BUFFER_SIZE):
            digest.update(chunk)
    return digest.hexdigest()


def file_description(path: Path) -> str:
    file_bin = shutil.which("file")
    if file_bin is None:
        return "file command unavailable"
    result = subprocess.run([file_bin, "-L", str(path)], text=True, capture_output=True, check=False)
    return result.stdout.strip() if result.returncode == 0 else result.stderr.strip()


def run_checked(command: list[str]) -> str:
    result = subprocess.run(command, text=True, capture_output=True, check=False)
    if result.returncode != 0:
        raise RuntimeError(
            f"command failed: {' '.join(command)}\nstdout={result.stdout}\nstderr={result.stderr}"
        )
    return result.stdout


def write_outputs(profile: dict[str, Any], args: argparse.Namespace) -> None:
    if args.json_output is not None:
        args.json_output.parent.mkdir(parents=True, exist_ok=True)
        args.json_output.write_text(json.dumps(profile, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def print_profile(profile: dict[str, Any]) -> None:
    print("[*] Claude Code native TLS profile")
    print(f"  status: {profile['status']}")
    print(f"  binary: {profile['binary']}")
    print(f"  arch: {profile['arch']}")
    print(f"  build_id: {profile['build_id']}")
    print(f"  sha256: {profile['sha256']}")
    package = profile.get("claude_package") or {}
    if package:
        print(f"  package: {package.get('name')} {package.get('version')} at {package.get('path')}")
        for native in package.get("native_packages", []):
            if native.get("installed"):
                print(f"  native package: {native['name']} {native['version']} at {native['path']}")
    print(f"  OpenSSL symbols: {profile['openssl_symbols']}")
    if profile["status"] == "supported":
        print(f"  resolver: {profile['resolver']}")
        print(f"  library: {profile['library']}")
        print(f"  symbol_map_path: {profile['symbol_map_path']}")
        if profile.get("symbol_map_detail"):
            print(f"  symbol_map_detail: {profile['symbol_map_detail']}")
    else:
        print(f"  reason: {profile.get('reason', 'unsupported runtime')}")
        print(f"  next_step: {profile.get('next_step', '')}")


if __name__ == "__main__":
    raise SystemExit(main())
