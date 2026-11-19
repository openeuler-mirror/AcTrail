from __future__ import annotations

import subprocess
import tempfile
import tomllib
import unittest
from pathlib import Path


REPO = Path(__file__).resolve().parents[5]
DEPLOY = REPO / "deploy/container-auto/deploy.sh"
OTEL_RENDERER = REPO / "deploy/container-auto/render-otel-http-config.sh"
OTEL_TEMPLATE = REPO / "examples/plugins/builtin/otel-http/otel-http.config.toml"


class ContainerAutoDeploymentTest(unittest.TestCase):
    def test_full_attribute_mode_requires_an_endpoint(self) -> None:
        completed = subprocess.run(
            [str(DEPLOY), "--otel-attribute-mode", "full", "--print-plan"],
            check=False,
            capture_output=True,
            text=True,
        )

        self.assertEqual(completed.returncode, 2)
        self.assertIn("requires --otel-endpoint", completed.stderr)

    def test_plan_rejects_an_invalid_collector_endpoint(self) -> None:
        completed = subprocess.run(
            [
                str(DEPLOY),
                "--otel-endpoint",
                "http://COLLECTOR_HOST:4318/v1/traces",
                "--print-plan",
            ],
            check=False,
            capture_output=True,
            text=True,
        )

        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("concrete Collector address", completed.stderr)

    def test_renderer_writes_safe_http_metadata_config(self) -> None:
        rendered = self._render_otel_config(
            "http://127.0.0.1:4318/v1/traces",
            "metadata-only",
        )

        self.assertEqual(rendered["endpoint"], "http://127.0.0.1:4318/v1/traces")
        self.assertTrue(rendered["allow_insecure"])
        self.assertEqual(rendered["attribute_mode"], "metadata-only")

    def test_renderer_writes_https_full_config(self) -> None:
        rendered = self._render_otel_config(
            "https://collector.example:4318/v1/traces",
            "full",
        )

        self.assertFalse(rendered["allow_insecure"])
        self.assertEqual(rendered["attribute_mode"], "full")

    def test_renderer_rejects_toml_injection(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            output = Path(temporary_directory) / "otel-http.config.toml"
            completed = subprocess.run(
                [
                    str(OTEL_RENDERER),
                    "--template",
                    str(OTEL_TEMPLATE),
                    "--output",
                    str(output),
                    "--endpoint",
                    'http://127.0.0.1:4318/v1/traces"\nqueue_capacity = 1',
                ],
                check=False,
                capture_output=True,
                text=True,
            )

            self.assertNotEqual(completed.returncode, 0)
            self.assertFalse(output.exists())

    def test_unknown_distribution_is_rejected(self) -> None:
        completed = subprocess.run(
            [str(DEPLOY), "--distro", "fedora", "--print-plan"],
            check=False,
            capture_output=True,
            text=True,
        )

        self.assertEqual(completed.returncode, 2)
        self.assertIn("unsupported distro: fedora", completed.stderr)

    def _render_otel_config(self, endpoint: str, attribute_mode: str) -> dict:
        with tempfile.TemporaryDirectory() as temporary_directory:
            output = Path(temporary_directory) / "otel-http.config.toml"
            completed = subprocess.run(
                [
                    str(OTEL_RENDERER),
                    "--template",
                    str(OTEL_TEMPLATE),
                    "--output",
                    str(output),
                    "--endpoint",
                    endpoint,
                    "--attribute-mode",
                    attribute_mode,
                ],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(completed.returncode, 0, completed.stderr)
            self.assertEqual(output.stat().st_mode & 0o777, 0o640)
            with output.open("rb") as stream:
                return tomllib.load(stream)


if __name__ == "__main__":
    unittest.main()
