#!/usr/bin/env python3
"""Prepare immutable guest assets and a local profile for Kata V2 tests."""

from __future__ import annotations

import argparse
import os
from pathlib import Path

from argparse_types import positive_int
from v2_artifacts import (
    ArtifactPreparer,
    PreparationInputs,
    default_tool_inputs,
    infer_runtime_path,
)


REPO = Path(__file__).resolve().parents[3]

def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(
        description=(
            "Build/reuse content-addressed AcTrail guest artifacts and write "
            "the machine-local V2 test profile. Build target/release first."
        )
    )
    result.add_argument(
        "--backend",
        choices=("stratovirt", "cloud-hypervisor"),
        default="stratovirt",
    )
    result.add_argument("--runtime", default="io.containerd.kata332.v2")
    result.add_argument("--kata-prefix", type=Path, default=Path("/opt/kata"))
    result.add_argument("--base-config-source", type=Path)
    result.add_argument("--data-config-source", type=Path)
    result.add_argument("--base-image-source", type=Path)
    result.add_argument("--data-image-source", type=Path)
    result.add_argument("--hypervisor", type=Path)
    result.add_argument("--base-kernel", type=Path)
    result.add_argument("--data-kernel", type=Path)
    result.add_argument("--virtiofsd", type=Path)
    result.add_argument(
        "--xiaoo",
        type=Path,
        default=(Path(os.environ["XIAOO_E2E_BINARY"]) if os.environ.get(
            "XIAOO_E2E_BINARY"
        ) else None),
        help="optional xiaoO executable included for the concurrency case",
    )
    result.add_argument(
        "--bin-dir",
        type=Path,
        default=REPO / "target/release",
    )
    result.add_argument(
        "--output-root",
        type=Path,
        default=REPO / "local/kata/artifacts",
    )
    result.add_argument(
        "--write-profile",
        type=Path,
        default=REPO / "local/kata/v2-test-profile.json",
    )
    result.add_argument(
        "--no-write-profile",
        action="store_true",
        help="prepare artifacts without creating the local test profile",
    )
    result.add_argument(
        "--workload-image",
        default="docker.io/library/actrail-openeuler-workload:24.09",
    )
    result.add_argument("--workload-image-archive", type=Path)
    result.add_argument(
        "--image-pull-policy",
        choices=("never", "missing", "always"),
        default="never",
    )
    result.add_argument(
        "--otel-endpoint",
        required=True,
        help="OTLP/HTTP traces URL reachable from inside the Kata Guest",
    )
    result.add_argument("--socket-gid", type=positive_int, default=39000)
    result.add_argument("--data-vcpus", type=positive_int, default=2)
    result.add_argument(
        "--check-only",
        action="store_true",
        help="validate the matching cache entry and profile without writing",
    )
    result.add_argument(
        "--skip-workload-image",
        action="store_true",
        help="do not inspect/import/pull the containerd workload image",
    )
    return result


def main(argv: list[str] | None = None) -> int:
    arguments = parser().parse_args(argv)
    try:
        default_config_name = {
            "stratovirt": "configuration-stratovirt.toml",
            "cloud-hypervisor": "configuration-clh.toml",
        }[arguments.backend]
        base_config = _source_path(
            arguments.base_config_source
            or arguments.kata_prefix
            / "share/defaults/kata-containers"
            / default_config_name
        )
        data_config = _source_path(arguments.data_config_source or base_config)
        inputs = PreparationInputs(
            repo=REPO.resolve(),
            bin_dir=_source_directory(arguments.bin_dir),
            output_root=_output_path(arguments.output_root),
            backend=arguments.backend,
            runtime=arguments.runtime,
            kata_prefix=_source_directory(arguments.kata_prefix),
            base_config_source=base_config,
            data_config_source=data_config,
            base_image_source=_source_path(
                arguments.base_image_source
                or infer_runtime_path(base_config, arguments.backend, "image")
            ),
            data_image_source=_source_path(
                arguments.data_image_source
                or infer_runtime_path(data_config, arguments.backend, "image")
            ),
            hypervisor=_source_path(
                arguments.hypervisor
                or infer_runtime_path(base_config, arguments.backend, "path")
            ),
            base_kernel=_source_path(
                arguments.base_kernel
                or infer_runtime_path(base_config, arguments.backend, "kernel")
            ),
            data_kernel=_source_path(
                arguments.data_kernel
                or infer_runtime_path(data_config, arguments.backend, "kernel")
            ),
            virtiofsd=_source_path(
                arguments.virtiofsd
                or infer_runtime_path(
                    base_config,
                    arguments.backend,
                    "virtio_fs_daemon",
                )
            ),
            xiaoo=(
                _source_path(arguments.xiaoo) if arguments.xiaoo is not None else None
            ),
            workload_image=arguments.workload_image,
            workload_image_archive=(
                _source_path(arguments.workload_image_archive)
                if arguments.workload_image_archive is not None
                else None
            ),
            image_pull_policy=arguments.image_pull_policy,
            otel_endpoint=arguments.otel_endpoint,
            socket_gid=arguments.socket_gid,
            data_vcpus=arguments.data_vcpus,
            tool_inputs=default_tool_inputs(REPO),
        )
        profile = (
            None
            if arguments.no_write_profile
            else _output_path(arguments.write_profile)
        )
        ArtifactPreparer(inputs).prepare(
            profile_path=profile,
            check_only=arguments.check_only,
            ensure_workload_image=not arguments.skip_workload_image,
        )
    except (OSError, RuntimeError, ValueError) as error:
        print(f"FAIL: {error}", file=os.sys.stderr)
        return 1
    return 0


def _source_path(path: Path) -> Path:
    resolved = _absolute(path)
    if not resolved.is_file():
        raise ValueError(f"input file does not exist: {resolved}")
    return resolved


def _source_directory(path: Path) -> Path:
    resolved = _absolute(path)
    if not resolved.is_dir():
        raise ValueError(f"input directory does not exist: {resolved}")
    return resolved


def _output_path(path: Path) -> Path:
    return _absolute(path)


def _absolute(path: Path) -> Path:
    if path.is_absolute():
        return path.resolve()
    return (Path.cwd() / path).resolve()


if __name__ == "__main__":
    raise SystemExit(main())
