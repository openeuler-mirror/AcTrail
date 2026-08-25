#!/usr/bin/env python3
"""Create an AcTrail VMM candidate from the matching Kata configuration."""

from __future__ import annotations

import argparse
import json
import os
import re
import tempfile
from pathlib import Path

from argparse_types import positive_int


EXPECTED_KATA_VERSION = "3.32.0"
BACKENDS = {
    "stratovirt": {
        "section": "stratovirt",
        "default_config": "configuration-stratovirt.toml",
        "ready_marker": "KATA_STRATOVIRT_CONFIG_READY",
        "requires_virtiofsd": True,
    },
    "cloud-hypervisor": {
        "section": "clh",
        "default_config": "configuration-clh.toml",
        "ready_marker": "KATA_CLOUD_HYPERVISOR_CONFIG_READY",
        "requires_virtiofsd": True,
    },
    "firecracker": {
        "section": "firecracker",
        "default_config": "configuration-fc.toml",
        "ready_marker": "KATA_FIRECRACKER_CONFIG_READY",
        "requires_virtiofsd": False,
    },
}

def executable_path(value: str) -> Path:
    path = Path(value)
    if not path.is_absolute():
        raise argparse.ArgumentTypeError(f"path must be absolute: {path}")
    if not path.is_file():
        raise argparse.ArgumentTypeError(f"file does not exist: {path}")
    if not os.access(path, os.X_OK):
        raise argparse.ArgumentTypeError(f"file is not executable: {path}")
    return path


def resolved_executable_path(value: str) -> Path:
    return executable_path(value).resolve()


def regular_path(value: str) -> Path:
    path = Path(value)
    if not path.is_absolute():
        raise argparse.ArgumentTypeError(f"path must be absolute: {path}")
    if not path.is_file():
        raise argparse.ArgumentTypeError(f"file does not exist: {path}")
    return path


def absolute_path(value: str) -> Path:
    path = Path(value)
    if not path.is_absolute():
        raise argparse.ArgumentTypeError(f"path must be absolute: {path}")
    return path


def toml_string(path: Path) -> str:
    return json.dumps(str(path))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--backend",
        choices=tuple(BACKENDS),
        default="stratovirt",
    )
    parser.add_argument("--kata-prefix", type=Path, default=Path("/opt/kata"))
    parser.add_argument("--base-config", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--hypervisor", type=executable_path, required=True)
    parser.add_argument("--jailer", type=resolved_executable_path)
    parser.add_argument("--kernel", type=regular_path, required=True)
    parser.add_argument("--image", type=regular_path, required=True)
    parser.add_argument(
        "--image-config-path",
        type=absolute_path,
        help=(
            "path written to the generated config when --image is a staging "
            "copy"
        ),
    )
    parser.add_argument("--virtiofsd", type=executable_path)
    parser.add_argument(
        "--default-vcpus",
        type=positive_int,
        help="override the candidate guest vCPU count",
    )
    parser.add_argument(
        "--debug",
        action="store_true",
        help="enable runtime, hypervisor, agent and guest-console diagnostics",
    )
    args = parser.parse_args()

    backend = BACKENDS[args.backend]
    if backend["requires_virtiofsd"] and args.virtiofsd is None:
        parser.error(f"--virtiofsd is required for {args.backend}")
    if args.backend == "firecracker" and args.jailer is None:
        parser.error("--jailer is required for firecracker")
    if args.backend != "firecracker" and args.jailer is not None:
        parser.error("--jailer is only valid for firecracker")

    prefix = args.kata_prefix.resolve()
    version_file = prefix / "VERSION"
    try:
        installed_version = version_file.read_text(encoding="utf-8").strip()
    except OSError as error:
        parser.error(f"cannot read Kata VERSION from {version_file}: {error}")
    if installed_version != EXPECTED_KATA_VERSION:
        parser.error(
            f"Kata prefix is {installed_version!r}, expected {EXPECTED_KATA_VERSION}"
        )

    hypervisor_section = f"hypervisor.{backend['section']}"
    base = args.base_config or (
        prefix
        / "share"
        / "defaults"
        / "kata-containers"
        / backend["default_config"]
    )
    if not base.is_file():
        parser.error(f"base {args.backend} config does not exist: {base}")
    if not args.output.is_absolute():
        parser.error("--output must be an absolute path")
    if args.output.exists() or args.output.is_symlink():
        parser.error(f"refusing to overwrite existing output: {args.output}")
    if not args.output.parent.is_dir():
        parser.error(f"output directory does not exist: {args.output.parent}")

    configured_image = args.image_config_path or args.image
    replacements = {
        (hypervisor_section, "path"): toml_string(args.hypervisor),
        (hypervisor_section, "kernel"): toml_string(args.kernel),
        (hypervisor_section, "image"): toml_string(configured_image),
        (hypervisor_section, "valid_hypervisor_paths"): (
            f"[{toml_string(args.hypervisor)}]"
        ),
    }
    if args.virtiofsd is not None and backend["requires_virtiofsd"]:
        replacements.update(
            {
                (hypervisor_section, "virtio_fs_daemon"): toml_string(
                    args.virtiofsd
                ),
                (hypervisor_section, "valid_virtio_fs_daemon_paths"): (
                    f"[{toml_string(args.virtiofsd)}]"
                ),
            }
        )
    if args.jailer is not None:
        replacements.update(
            {
                (hypervisor_section, "jailer_path"): toml_string(args.jailer),
                (hypervisor_section, "valid_jailer_paths"): (
                    f"[{toml_string(args.jailer)}]"
                ),
            }
        )
    if args.debug:
        replacements.update(
            {
                (hypervisor_section, "enable_debug"): "true",
                ("agent.kata", "enable_debug"): "true",
                ("agent.kata", "debug_console_enabled"): "true",
                ("runtime", "enable_debug"): "true",
            }
        )
    if args.default_vcpus is not None:
        replacements[(hypervisor_section, "default_vcpus")] = str(
            args.default_vcpus
        )

    section = ""
    replaced = set()
    output_lines = [
        "# AcTrail generated candidate; do not edit the Kata release default.\n",
        f"# kata_version={EXPECTED_KATA_VERSION} base={base}\n",
    ]
    assignment = re.compile(r"^(\s*)([A-Za-z0-9_]+)(\s*=).*$")
    section_header = re.compile(r"^\s*\[([^]]+)]\s*$")
    try:
        source_lines = base.read_text(encoding="utf-8").splitlines(keepends=True)
    except OSError as error:
        parser.error(f"cannot read base config {base}: {error}")

    for line in source_lines:
        header_match = section_header.match(line)
        if header_match:
            section = header_match.group(1)
            output_lines.append(line)
            continue
        assignment_match = assignment.match(line)
        if assignment_match:
            key = assignment_match.group(2)
            replacement_key = (section, key)
            if replacement_key in replacements:
                indent, _, equals = assignment_match.groups()
                output_lines.append(
                    f"{indent}{key}{equals} {replacements[replacement_key]}\n"
                )
                replaced.add(replacement_key)
                continue
        output_lines.append(line)

    missing = sorted(set(replacements) - replaced)
    if missing:
        formatted = ", ".join(f"[{section}].{key}" for section, key in missing)
        parser.error(f"base config is missing required settings: {formatted}")

    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{args.output.name}.",
        dir=args.output.parent,
        text=True,
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as destination:
            destination.writelines(output_lines)
            destination.flush()
            os.fsync(destination.fileno())
        os.chmod(temporary, 0o644)
        os.replace(temporary, args.output)
    finally:
        if temporary.exists():
            temporary.unlink()

    print(backend["ready_marker"])
    print(f"backend={args.backend}")
    print(f"kata_version={installed_version}")
    print(f"output={args.output}")
    print(f"hypervisor={args.hypervisor}")
    print(f"jailer={args.jailer or 'not-configured'}")
    print(f"kernel={args.kernel}")
    print(f"image_source={args.image}")
    print(f"image={configured_image}")
    print(f"virtiofsd={args.virtiofsd or 'not-configured'}")
    print(f"default_vcpus={args.default_vcpus or 'release-default'}")
    print(f"debug={str(args.debug).lower()}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
