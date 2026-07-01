#!/usr/bin/env python3
"""Clean runtime artifacts produced by docs/examples transfer tests."""

from __future__ import annotations

import argparse
import re
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path


SAFE_TMP_PREFIX = "/tmp/actrail-"
STORAGE_SQLITE_CLEAN_RE = re.compile(r"^- Clear storage_sqlite_path (?P<path>\S+)\b")
CLEANABLE_PATH_KEYS = {
    "cert_directory",
    "fifo_path",
    "file_path",
    "mkdir_path",
    "mmap_path",
    "otel_output_path",
    "rename_source_path",
    "rename_target_path",
    "resolved_config_path",
    "rmdir_path",
    "truncate_path",
    "unlink_path",
}


@dataclass(frozen=True)
class ExampleCleanup:
    name: str
    operator_configs: tuple[Path, ...] = ()
    auxiliary_configs: tuple[Path, ...] = ()


EXAMPLES = (
    ExampleCleanup(
        name="quick-start",
        operator_configs=(Path("docs/examples/01.quick-start/operator.conf"),),
    ),
    ExampleCleanup(
        name="llm-http1",
        operator_configs=(
            Path(
                "docs/examples/02.llm-http-payload-capture/external-openai-compatible/"
                "http1-operator.conf"
            ),
        ),
    ),
    ExampleCleanup(
        name="llm-http2",
        operator_configs=(
            Path(
                "docs/examples/02.llm-http-payload-capture/external-openai-compatible/"
                "http2-operator.conf"
            ),
        ),
    ),
    ExampleCleanup(
        name="http2-local",
        operator_configs=(
            Path("docs/examples/02.llm-http-payload-capture/http2-local/operator.conf"),
        ),
        auxiliary_configs=(
            Path("docs/examples/02.llm-http-payload-capture/http2-local/workload.conf"),
        ),
    ),
    ExampleCleanup(
        name="extended-observation",
        operator_configs=(
            Path("docs/examples/03.extended-observation-e2e/operator.conf"),
        ),
        auxiliary_configs=(
            Path("docs/examples/03.extended-observation-e2e/workload.conf"),
        ),
    ),
    ExampleCleanup(
        name="http-payload",
        operator_configs=(Path("docs/examples/05.http-payload-unified/operator.conf"),),
    ),
    ExampleCleanup(
        name="xiaoo-tls",
        operator_configs=(
            Path("docs/examples/06.xiaoo-tls-capture/operator.conf"),
        ),
    ),
    ExampleCleanup(
        name="xiaoo-claude",
        operator_configs=(
            Path("docs/examples/07.xiaoo-claude-agent-invocation/operator.conf"),
        ),
        auxiliary_configs=(
            Path("docs/examples/07.xiaoo-claude-agent-invocation/workload.conf"),
        ),
    ),
    ExampleCleanup(
        name="full-monitor",
        operator_configs=(
            Path("docs/examples/08.full-monitor-validation/operator.conf"),
        ),
    ),
    ExampleCleanup(
        name="java-langchain4j-agent",
        operator_configs=(
            Path("docs/examples/10.java-langchain4j-agent/operator.conf"),
        ),
        auxiliary_configs=(
            Path("docs/examples/10.java-langchain4j-agent/workload.conf"),
        ),
    ),
)


def main() -> int:
    args = parse_args()
    repo = Path.cwd()
    actrailctl = repo / args.bin_dir / "actrailctl"
    if not actrailctl.exists():
        raise RuntimeError(f"missing {actrailctl}; build with cargo build --release")

    selected = select_examples(args.example)
    for example in selected:
        print(f"clean example={example.name}", flush=True)
        for config in example.operator_configs:
            clean_operator_config(actrailctl, resolve_path(repo, config))
        for config in example.auxiliary_configs:
            clean_auxiliary_config(resolve_path(repo, config))
    return 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", default="target/release")
    parser.add_argument(
        "--example",
        choices=("all",) + tuple(example.name for example in EXAMPLES),
        default="all",
        help="example artifact set to clean",
    )
    return parser.parse_args()


def select_examples(name: str) -> tuple[ExampleCleanup, ...]:
    if name == "all":
        return EXAMPLES
    return tuple(example for example in EXAMPLES if example.name == name)


def resolve_path(repo: Path, path: Path) -> Path:
    if path.is_absolute():
        return path
    return repo / path


def clean_operator_config(actrailctl: Path, config: Path) -> None:
    if not config.exists():
        print(f"skipped missing operator config {config}")
        return
    completed = subprocess.run(
        [str(actrailctl), "clean", "--config", str(config)],
        text=True,
        capture_output=True,
        check=False,
    )
    print(completed.stdout, end="")
    print(completed.stderr, end="", file=sys.stderr)
    if completed.returncode != 0:
        raise subprocess.CalledProcessError(
            completed.returncode,
            completed.args,
            output=completed.stdout,
            stderr=completed.stderr,
        )
    for sqlite_path in cleaned_sqlite_paths(completed.stdout):
        remove_sqlite_sidecars(sqlite_path)


def clean_auxiliary_config(config: Path) -> None:
    if not config.exists():
        print(f"skipped missing auxiliary config {config}")
        return
    values = read_config(config)
    for key, value in values.items():
        if should_clean_key(key):
            remove_safe_tmp_path(Path(value))


def read_config(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    lines = path.read_text(encoding="utf-8").splitlines()
    for line_number, raw in enumerate(lines, start=1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        key, separator, value = line.partition("=")
        if not separator:
            raise RuntimeError(f"{path}:{line_number}: expected key = value")
        values[key.strip()] = value.strip()
    return values


def should_clean_key(key: str) -> bool:
    return key in CLEANABLE_PATH_KEYS


def remove_safe_tmp_path(path: Path) -> None:
    if not path.is_absolute():
        return
    raw = str(path)
    if not raw.startswith(SAFE_TMP_PREFIX):
        print(f"skipped non-AcTrail tmp path {path}")
        return
    if path.is_dir():
        shutil.rmtree(path)
        print(f"removed directory {path}")
        return
    if path.exists():
        path.unlink()
        print(f"removed file {path}")
        return
    print(f"skipped missing auxiliary path {path}")


def cleaned_sqlite_paths(output: str) -> list[Path]:
    paths: list[Path] = []
    for line in output.splitlines():
        match = STORAGE_SQLITE_CLEAN_RE.match(line)
        if match:
            paths.append(Path(match.group("path")))
    return paths


def remove_sqlite_sidecars(path: Path) -> None:
    for suffix in ("-wal", "-shm"):
        remove_safe_tmp_path(Path(str(path) + suffix))


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"docs example cleanup failed: {error}", file=sys.stderr)
        raise SystemExit(1)
