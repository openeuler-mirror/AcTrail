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
DEFAULT_PROFILE = REPO / "local/kata/v2-test-profile.json"
CURRENT_LINKS = REPO / "local/kata/current"

def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(
        description=(
            "Validate/reuse AcTrail guest artifacts, rebuild missing or invalid "
            "artifacts, and write "
            "the machine-local V2 test profile. Build target/release first."
        )
    )
    result.add_argument(
        "--backend",
        choices=("stratovirt", "cloud-hypervisor", "firecracker"),
        default="stratovirt",
    )
    result.add_argument("--runtime", default="io.containerd.kata332.v2")
    result.add_argument("--kata-prefix", type=Path, default=Path("/opt/kata"))
    result.add_argument("--base-config-source", type=Path)
    result.add_argument("--data-config-source", type=Path)
    image_help = (
        "Guest source image copied and injected with AcTrail, for example the "
        "image built by guest/build-openeuler-image.sh; it is not taken from "
        "the Kata config"
    )
    result.add_argument(
        "--base-image-source", type=Path, required=True, help=image_help
    )
    result.add_argument(
        "--data-image-source", type=Path, required=True, help=image_help
    )
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
        help=(
            "optional xiaoO executable included for concurrency and "
            "execution-isolation cases"
        ),
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
        help=(
            "write the machine-local V2 test profile to this path (default: "
            "local/kata/v2-test-profile.json; with --activate no profile is "
            "written unless this option is given)"
        ),
    )
    result.add_argument(
        "--no-write-profile",
        action="store_true",
        help="prepare artifacts without creating the local test profile",
    )
    result.add_argument(
        "--activate",
        action="store_true",
        help=(
            "after successful preparation, make this artifact the one in use: "
            "point local/kata/current/<backend>-<network|vsock> at it and remove "
            "unused managed artifacts; artifacts used by a current link, an input "
            "path, a local V2 test profile or a running process are kept, and "
            "nothing is removed when usage cannot be determined; the V2 test "
            "profile is not written unless --write-profile is given (used by "
            "docs/operations/deployment/virtual-container.md)"
        ),
    )
    result.add_argument(
        "--workload-image",
        default="docker.io/library/actrail-openeuler-workload:24.03",
    )
    result.add_argument("--workload-image-archive", type=Path)
    result.add_argument(
        "--image-pull-policy",
        choices=("never", "missing", "always"),
        default="never",
    )
    result.add_argument(
        "--otel-endpoint",
        help=(
            "optional OTLP/HTTP traces URL reachable from inside the Kata Guest; "
            "when omitted, keep observations in Guest-local SQLite only"
        ),
    )
    result.add_argument(
        "--egress-mode",
        choices=("network", "vsock-bridge"),
        default="network",
        help="export path used when --otel-endpoint is set",
    )
    result.add_argument(
        "--otel-attribute-mode",
        choices=("metadata-only", "full"),
        default="metadata-only",
        help=(
            "otel-http attribute_mode written into the Guest; full also exports "
            "command lines, model names, token counts and message previews "
            "(requires --otel-endpoint)"
        ),
    )
    result.add_argument(
        "--collector-traces-file",
        type=Path,
        help=(
            "Host Collector file exporter output; written into the V2 test "
            "profile so the regression asserts that spans of the run reached "
            "the Collector (requires --otel-endpoint)"
        ),
    )
    result.add_argument(
        "--cni-config",
        type=Path,
        help=(
            "single-plugin CNI configuration for host/cni-netns.sh; written "
            "into the V2 test profile so each regression VM gets a Guest "
            "interface (requires --otel-endpoint with --egress-mode network)"
        ),
    )
    result.add_argument("--socket-gid", type=positive_int, default=39000)
    result.add_argument("--data-vcpus", type=positive_int, default=2)
    result.add_argument(
        "--with-sandbox-observer",
        action="store_true",
        help=(
            "install actrail-sb as a Guest system service in both images"
        ),
    )
    result.add_argument(
        "--check-only",
        action="store_true",
        help="validate matching artifacts/profile without rebuilding, activating or pruning",
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
            "firecracker": "configuration-fc.toml",
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
            base_image_source=_source_path(arguments.base_image_source),
            data_image_source=_source_path(arguments.data_image_source),
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
            virtiofsd=(
                _source_path(arguments.virtiofsd)
                if arguments.virtiofsd is not None
                else (
                    None
                    if arguments.backend == "firecracker"
                    else _source_path(
                        infer_runtime_path(
                            base_config,
                            arguments.backend,
                            "virtio_fs_daemon",
                        )
                    )
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
            egress_mode=arguments.egress_mode,
            otel_attribute_mode=arguments.otel_attribute_mode,
            socket_gid=arguments.socket_gid,
            data_vcpus=arguments.data_vcpus,
            sandbox_observer=arguments.with_sandbox_observer,
            tool_inputs=default_tool_inputs(REPO),
        )
        if arguments.no_write_profile:
            profile = None
        elif arguments.write_profile is not None:
            profile = _output_path(arguments.write_profile)
        elif arguments.activate:
            # Deployment keeps the regression profile untouched.
            profile = None
        else:
            profile = DEFAULT_PROFILE
        ArtifactPreparer(inputs).prepare(
            profile_path=profile,
            profile_environment=_export_evidence_environment(arguments),
            # Every local profile, including the per-backend ones the
            # execution-isolation cases write next to the default.
            keep_profiles=sorted(
                DEFAULT_PROFILE.parent.glob("v2-test-profile*.json")
            ),
            check_only=arguments.check_only,
            ensure_workload_image=not arguments.skip_workload_image,
            current_root=CURRENT_LINKS if arguments.activate else None,
        )
    except (OSError, RuntimeError, ValueError) as error:
        print(f"FAIL: {error}", file=os.sys.stderr)
        return 1
    return 0


def _export_evidence_environment(arguments: argparse.Namespace) -> dict[str, str]:
    """Profile settings that make the regression assert Collector delivery."""
    environment: dict[str, str] = {}
    traces_file = arguments.collector_traces_file
    cni_config = arguments.cni_config
    if traces_file is None and cni_config is None:
        return environment
    if arguments.otel_endpoint is None:
        raise ValueError(
            "--collector-traces-file and --cni-config require --otel-endpoint"
        )
    if traces_file is not None:
        environment["VIRTUAL_CONTAINER_E2E_COLLECTOR_TRACES_FILE"] = str(
            _output_path(traces_file)
        )
    if cni_config is not None:
        if arguments.egress_mode != "network":
            raise ValueError("--cni-config requires --egress-mode network")
        environment["ACTRAIL_TEST_CNI_CONFIG"] = str(_source_path(cni_config))
        environment["VIRTUAL_CONTAINER_E2E_NETWORK_NAMESPACE_HELPER"] = str(
            REPO / "deploy/virtual-container/host/cni-netns.sh"
        )
    return environment


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
