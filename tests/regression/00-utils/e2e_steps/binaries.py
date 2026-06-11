"""Binary checks shared by direct E2E regression steps."""

from __future__ import annotations

from pathlib import Path

from evidence import expected_found_detail
from model import CaseResult

from .checks import run_step


def require_actrail_binaries(result: CaseResult, module, bin_dir: Path) -> tuple[Path, Path, Path]:
    actraild = require_actrail_binary(result, module, bin_dir, "actraild")
    actrailctl = require_actrail_binary(result, module, bin_dir, "actrailctl")
    actrailviewer = require_actrail_binary(result, module, bin_dir, "actrailviewer")
    return actraild, actrailctl, actrailviewer


def require_actrailweb_binary(result: CaseResult, module, bin_dir: Path) -> Path:
    return require_actrail_binary(result, module, bin_dir, "actrailweb")


def require_actrail_binary(result: CaseResult, module, bin_dir: Path, name: str) -> Path:
    return run_step(
        result,
        f"{name} binary",
        lambda: module.require_binary(bin_dir, name),
        lambda path: expected_found_detail(f"{name} release binary exists", [f"path={path}"]),
        f"{name} release binary is available",
    )
