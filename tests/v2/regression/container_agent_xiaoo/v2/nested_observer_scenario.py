#!/usr/bin/env python3
"""Run the real container xiaoO scenario below a nested observer PID namespace."""

from __future__ import annotations

import argparse
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import zlib
from pathlib import Path


CASE_DIR = Path(__file__).resolve().parent
REPO = Path(__file__).resolve().parents[5]


class NestedDockerRuntime:
    def __init__(self, runtime: Path):
        self._runtime = runtime
        self._socket = runtime / "docker.sock"
        self._log = runtime / "dockerd.stderr"
        self._process: subprocess.Popen[str] | None = None

    @property
    def environment(self) -> dict[str, str]:
        environment = os.environ.copy()
        environment["DOCKER_HOST"] = f"unix://{self._socket}"
        environment["ACTRAIL_REQUIRE_NESTED_PID_IDENTITY"] = "1"
        return environment

    def start(self, timeout: float) -> None:
        log = self._log.open("w", encoding="utf-8")
        self._process = subprocess.Popen(
            [
                "dockerd",
                "--host",
                f"unix://{self._socket}",
                "--data-root",
                str(self._runtime / "data"),
                "--exec-root",
                str(self._runtime / "exec"),
                "--pidfile",
                str(self._runtime / "docker.pid"),
                "--storage-driver",
                "vfs",
                "--iptables=false",
                "--ip-masq=false",
                "--bridge=none",
            ],
            stdout=log,
            stderr=log,
            text=True,
            start_new_session=True,
        )
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if self._process.poll() is not None:
                break
            result = subprocess.run(
                ["docker", "info", "--format", "{{.ServerVersion}}"],
                env=self.environment,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
            if result.returncode == 0:
                return
            time.sleep(0.1)
        raise RuntimeError(
            "nested Docker daemon did not become ready:\n"
            + self._log.read_text(encoding="utf-8", errors="replace")[-16_000:]
        )

    def load_image(self, archive: Path) -> None:
        run_checked(
            ["docker", "image", "load", "--input", str(archive)],
            environment=self.environment,
        )

    def stop(self) -> None:
        if self._process is None or self._process.poll() is not None:
            return
        try:
            os.killpg(self._process.pid, signal.SIGTERM)
        except ProcessLookupError:
            return
        try:
            self._process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(self._process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            self._process.wait(timeout=5)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", default="target/release")
    parser.add_argument(
        "--image",
        default="openeuler/openeuler:24.03-lts-sp3",
    )
    parser.add_argument("--xiaoo-bin", default="/root/.cargo/bin/xiaoo")
    parser.add_argument("--rebuild-image", action="store_true")
    parser.add_argument("--keep-runtime", action="store_true")
    parser.add_argument("--keep-runtime-on-failure", action="store_true")
    parser.add_argument("--child", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--runtime", type=Path, help=argparse.SUPPRESS)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.child:
        if args.runtime is None:
            raise RuntimeError("nested observer child requires --runtime")
        return run_child(args, args.runtime)
    return run_parent(args)


def run_parent(args: argparse.Namespace) -> int:
    runtime = Path(tempfile.mkdtemp(prefix="actrail-nested-observer.", dir="/tmp"))
    succeeded = False
    try:
        archive = runtime / "container-agent-xiaoo-image.tar"
        image_reference = ensure_host_agent_image(args.image, args.rebuild_image, runtime)
        run_checked(["docker", "image", "save", "--output", str(archive), image_reference])
        command = [
            "unshare",
            "--pid",
            "--net",
            "--fork",
            "--mount-proc",
            "--",
            sys.executable,
            str(Path(__file__).resolve()),
            "--child",
            "--runtime",
            str(runtime),
            "--bin-dir",
            str(Path(args.bin_dir).resolve()),
            "--image",
            args.image,
            "--xiaoo-bin",
            str(Path(args.xiaoo_bin).resolve()),
        ]
        if args.rebuild_image:
            command.append("--rebuild-image")
        completed = subprocess.run(command, cwd=REPO, check=False)
        succeeded = completed.returncode == 0
        return completed.returncode
    finally:
        if args.keep_runtime or (args.keep_runtime_on_failure and not succeeded):
            print(
                f"nested_observer_runtime_preserved={runtime} succeeded={succeeded}",
                file=sys.stderr,
            )
        else:
            shutil.rmtree(runtime, ignore_errors=True)


def run_child(args: argparse.Namespace, runtime: Path) -> int:
    docker = NestedDockerRuntime(runtime)
    try:
        run_checked(["ip", "link", "set", "lo", "up"])
        docker.start(timeout=30.0)
        docker.load_image(runtime / "container-agent-xiaoo-image.tar")
        command = [
            sys.executable,
            str(CASE_DIR / "xiaoo_scenario.py"),
            "--bin-dir",
            args.bin_dir,
            "--image",
            args.image,
            "--xiaoo-bin",
            args.xiaoo_bin,
            "--keep-runtime-on-failure",
        ]
        if args.rebuild_image:
            command.append("--rebuild-image")
        return subprocess.run(
            command,
            cwd=REPO,
            env=docker.environment,
            check=False,
        ).returncode
    finally:
        docker.stop()


def ensure_host_agent_image(base_image: str, rebuild: bool, runtime: Path) -> str:
    dockerfile = CASE_DIR / "Dockerfile"
    cache_key = zlib.crc32(b"container-agent-xiaoo-runtime-v2\0")
    cache_key = zlib.crc32(base_image.encode("utf-8"), cache_key)
    cache_key = zlib.crc32(dockerfile.read_bytes(), cache_key)
    reference = f"actrail/container-agent-xiaoo:runtime-v2-{cache_key:08x}"
    exists = subprocess.run(
        ["docker", "image", "inspect", reference],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    ).returncode == 0
    if exists and not rebuild:
        return reference
    context = runtime / "image-context"
    context.mkdir()
    run_checked(
        [
            "docker",
            "build",
            "-q",
            "--network",
            "host",
            "-f",
            str(dockerfile),
            "--build-arg",
            f"BASE_IMAGE={base_image}",
            "-t",
            reference,
            str(context),
        ]
    )
    return reference


def run_checked(
    command: list[str],
    *,
    environment: dict[str, str] | None = None,
) -> str:
    result = subprocess.run(
        command,
        env=environment,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        diagnostic = (result.stderr or result.stdout).strip()
        raise RuntimeError(
            f"command failed exit={result.returncode}: {' '.join(command)}\n{diagnostic}"
        )
    return result.stdout


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"nested-observer xiaoO E2E failed: {error}", file=sys.stderr)
        raise SystemExit(1)
