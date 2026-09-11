#!/usr/bin/env python3
"""Run only container acceptance; no system-wide binary/dependency installation."""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))
from tests.v2.common.core import TestCaseInputs
from tests.v2.regression.resource_metrics_container.case import ContainerAcceptance


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, default=REPO / "target/release")
    parser.add_argument("--image", required=True, help="existing local image with sh and sleep; never pulled")
    args = parser.parse_args()
    binaries = args.bin_dir.resolve()
    if (os.geteuid() != 0 or not shutil.which("docker")
            or not Path("/sys/fs/cgroup/cgroup.controllers").is_file()
            or any(not (binaries / name).is_file() for name in ("actraild", "actrailctl", "actrailviewer"))):
        print("SKIPPED: root, Docker, cgroup v2 and daemon/CLI/viewer binaries are required")
        return 77
    if subprocess.run(("docker", "image", "inspect", args.image), stdout=subprocess.DEVNULL,
                      stderr=subprocess.DEVNULL).returncode:
        print("SKIPPED: local test image or Docker daemon unavailable")
        return 77
    with tempfile.TemporaryDirectory(prefix="actrail-container-acceptance-") as directory:
        case = ContainerAcceptance(TestCaseInputs(repo=REPO, bin_dir=binaries, work_dir=Path(directory)))
        try:
            case.exercise(args.image)
        except Exception:
            log = Path(directory) / "daemon-output.log"
            if log.exists():
                print(log.read_text()[-8000:], file=sys.stderr)
            raise
    print("PASSED: container counters, read-only controls, restart recovery and finalization")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
