#!/usr/bin/env python3
"""Contract checks for the machine-local V2 test profile loader."""

from __future__ import annotations

import json
import os
import sys
import tempfile
from pathlib import Path
from unittest.mock import patch


REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.v2.regression.local_profile import (  # noqa: E402
    load_local_test_profile,
)


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="actrail-v2-profile.") as raw_dir:
        work_dir = Path(raw_dir)
        profile_path = work_dir / "profile.json"
        profile_path.write_text(
            json.dumps(
                {
                    "format": 1,
                    "path_prepend": ["${REPO}/local/bin", "/opt/kata/bin"],
                    "environment": {
                        "CTR_RUNTIME": "from-profile",
                        "XIAOO_E2E_BINARY": "${PROFILE_DIR}/bin/xiaoo",
                    },
                }
            ),
            encoding="utf-8",
        )
        with patch.dict(
            os.environ,
            {"PATH": "/usr/bin", "CTR_RUNTIME": "from-shell"},
            clear=True,
        ):
            loaded = load_local_test_profile(REPO, profile_path)
            assert loaded == profile_path.resolve()
            assert os.environ["CTR_RUNTIME"] == "from-shell"
            assert os.environ["XIAOO_E2E_BINARY"] == str(
                profile_path.resolve().parent / "bin/xiaoo"
            )
            assert os.environ["PATH"].split(os.pathsep) == [
                str(REPO / "local/bin"),
                "/opt/kata/bin",
                "/usr/bin",
            ]

        profile_path.write_text(
            json.dumps(
                {
                    "format": 1,
                    "environment": {"UNSAFE_UNSUPPORTED_KEY": "value"},
                }
            ),
            encoding="utf-8",
        )
        with patch.dict(os.environ, {}, clear=True):
            try:
                load_local_test_profile(REPO, profile_path)
            except ValueError as error:
                assert "unsupported environment key" in str(error)
            else:
                raise AssertionError("unsupported profile key was accepted")

        profile_path.write_text(
            json.dumps(
                {
                    "format": 2,
                    "environment": {
                        "VIRTUAL_CONTAINER_E2E_ARTIFACT_MANIFEST": (
                            "${PROFILE_DIR}/artifacts/digest/manifest.json"
                        ),
                        "VIRTUAL_CONTAINER_E2E_IMAGE_PULL_POLICY": "never",
                        "VIRTUAL_CONTAINER_E2E_FIRECRACKER_CONFIG": (
                            "${PROFILE_DIR}/configuration-fc-base.toml"
                        ),
                        "VIRTUAL_CONTAINER_E2E_FIRECRACKER_DATA_CONFIG": (
                            "${PROFILE_DIR}/configuration-fc-data.toml"
                        ),
                    },
                }
            ),
            encoding="utf-8",
        )
        with patch.dict(os.environ, {}, clear=True):
            load_local_test_profile(REPO, profile_path)
            assert os.environ[
                "VIRTUAL_CONTAINER_E2E_ARTIFACT_MANIFEST"
            ] == str(profile_path.resolve().parent / "artifacts/digest/manifest.json")
            assert os.environ["VIRTUAL_CONTAINER_E2E_IMAGE_PULL_POLICY"] == "never"
            assert os.environ[
                "VIRTUAL_CONTAINER_E2E_FIRECRACKER_CONFIG"
            ] == str(profile_path.resolve().parent / "configuration-fc-base.toml")
            assert os.environ[
                "VIRTUAL_CONTAINER_E2E_FIRECRACKER_DATA_CONFIG"
            ] == str(profile_path.resolve().parent / "configuration-fc-data.toml")

        profile_path.write_text(
            json.dumps(
                {
                    "format": 2,
                    "environment": {
                        "EXECUTION_ISOLATION_FIRECRACKER_E2E_KERNEL_IMAGE": (
                            "/tmp/direct-firecracker-kernel"
                        )
                    },
                }
            ),
            encoding="utf-8",
        )
        with patch.dict(os.environ, {}, clear=True):
            try:
                load_local_test_profile(REPO, profile_path)
            except ValueError as error:
                assert "unsupported environment key" in str(error)
            else:
                raise AssertionError("direct Firecracker profile key was accepted")

    print("LOCAL_TEST_PROFILE_CONTRACT_OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
