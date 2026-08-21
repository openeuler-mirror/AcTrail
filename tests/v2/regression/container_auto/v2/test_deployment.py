from __future__ import annotations

import os
import shutil
import subprocess
import tempfile
import tomllib
import unittest
from pathlib import Path


REPO = Path(__file__).resolve().parents[5]
DEPLOY = REPO / "deploy/container-auto/deploy.sh"
WAIT_SERVICE = REPO / "deploy/container-auto/wait-service-active.sh"
HOST_CONFIG = REPO / "deploy/container-auto/container-auto.conf"
OTEL_RENDERER = REPO / "deploy/container-auto/render-otel-http-config.sh"
OTEL_TEMPLATE = REPO / "examples/plugins/builtin/otel-http/otel-http.config.toml"


class ContainerAutoDeploymentTest(unittest.TestCase):
    def test_host_config_uses_absolute_plugin_discovery_directory(self) -> None:
        with HOST_CONFIG.open("rb") as stream:
            config = tomllib.load(stream)

        directory = config["plugins"]["discovery"]["directory"]
        self.assertTrue(Path(directory).is_absolute(), directory)
        self.assertEqual(directory, "/etc/actrail/plugins")

    def test_service_wait_rejects_activating_state(self) -> None:
        completed = self._run_service_wait("activating")

        self.assertEqual(completed.returncode, 1)
        self.assertIn("did not become active", completed.stderr)

    def test_service_wait_accepts_only_active_state(self) -> None:
        completed = self._run_service_wait("active")

        self.assertEqual(completed.returncode, 0, completed.stderr)

    def test_default_plan_uses_openeuler_2403(self) -> None:
        completed = subprocess.run(
            [str(DEPLOY), "--print-plan"],
            check=False,
            capture_output=True,
            text=True,
        )

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("distro=openeuler", completed.stdout)
        self.assertIn(
            "base_image=openeuler/openeuler:24.03-lts-sp3",
            completed.stdout,
        )
        self.assertIn(
            "workload_image=actrail/container-auto:openeuler-24.03",
            completed.stdout,
        )
        self.assertIn("otel_endpoint=disabled", completed.stdout)

    def test_ubuntu_plan_uses_ubuntu_2404(self) -> None:
        completed = subprocess.run(
            [str(DEPLOY), "--distro", "ubuntu", "--print-plan"],
            check=False,
            capture_output=True,
            text=True,
        )

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("distro=ubuntu", completed.stdout)
        self.assertIn("base_image=ubuntu:24.04", completed.stdout)
        self.assertIn(
            "workload_image=actrail/container-auto:ubuntu-24.04",
            completed.stdout,
        )

    def test_plan_accepts_explicit_images_and_release_directory(self) -> None:
        completed = subprocess.run(
            [
                str(DEPLOY),
                "--distro",
                "ubuntu",
                "--base-image",
                "registry.example/ubuntu:24.04",
                "--image",
                "registry.example/actrail:ubuntu",
                "--bin-dir",
                "/opt/actrail-release",
                "--pull-policy",
                "never",
                "--print-plan",
            ],
            check=False,
            capture_output=True,
            text=True,
        )

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("base_image=registry.example/ubuntu:24.04", completed.stdout)
        self.assertIn(
            "workload_image=registry.example/actrail:ubuntu",
            completed.stdout,
        )
        self.assertIn("release_source=/opt/actrail-release", completed.stdout)
        self.assertIn("pull_policy=never", completed.stdout)

    def test_plan_accepts_host_collector_endpoint(self) -> None:
        endpoint = "http://127.0.0.1:4318/v1/traces"
        completed = subprocess.run(
            [
                str(DEPLOY),
                "--otel-endpoint",
                endpoint,
                "--otel-attribute-mode",
                "full",
                "--print-plan",
            ],
            check=False,
            capture_output=True,
            text=True,
        )

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn(f"otel_endpoint={endpoint}", completed.stdout)
        self.assertIn("otel_attribute_mode=full", completed.stdout)

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

    def test_deploy_smoke_runs_a_real_trace_and_checks_otlp_delivery(self) -> None:
        source = DEPLOY.read_text(encoding="utf-8")

        self.assertIn('"$WORKLOAD_IMAGE" /bin/true', source)
        self.assertIn("metric.otel_http.successful_batches", source)
        self.assertIn("otel_delivery=verified", source)

    def test_unknown_distribution_is_rejected(self) -> None:
        completed = subprocess.run(
            [str(DEPLOY), "--distro", "fedora", "--print-plan"],
            check=False,
            capture_output=True,
            text=True,
        )

        self.assertEqual(completed.returncode, 2)
        self.assertIn("unsupported distro: fedora", completed.stderr)

    def _run_service_wait(self, active_state: str) -> subprocess.CompletedProcess[str]:
        with tempfile.TemporaryDirectory() as temporary_directory:
            fake_systemctl = Path(temporary_directory) / "systemctl"
            fake_systemctl.write_text(
                "#!/usr/bin/env bash\n"
                "if [[ $1 == show ]]; then\n"
                f"  echo {active_state}\n"
                "  exit 0\n"
                "fi\n"
                "exit 0\n",
                encoding="utf-8",
            )
            fake_systemctl.chmod(0o755)
            return subprocess.run(
                [str(WAIT_SERVICE), "actraild.service"],
                check=False,
                capture_output=True,
                text=True,
                env={
                    "PATH": "/usr/bin:/bin",
                    "ACTRAIL_SYSTEMCTL": str(fake_systemctl),
                    "ACTRAIL_SERVICE_READY_ATTEMPTS": "2",
                    "ACTRAIL_SERVICE_READY_INTERVAL": "0",
                    "ACTRAIL_JOURNALCTL": "/usr/bin/true",
                },
            )

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


class InstallBuildDependenciesTest(unittest.TestCase):
    def test_missing_node_modules_does_not_reinstall_ready_native_packages(
        self,
    ) -> None:
        bash_version = subprocess.run(
            ["bash", "-c", "printf '%s' \"${BASH_VERSINFO[0]}\""],
            check=True,
            capture_output=True,
            text=True,
        ).stdout
        if int(bash_version) < 4:
            self.skipTest("install-build-deps.sh requires Bash 4 or newer")

        with tempfile.TemporaryDirectory(prefix="actrail-build-deps.") as raw_dir:
            fixture = Path(raw_dir)
            scripts = fixture / "scripts"
            frontend = fixture / "crates/apps/web/frontend"
            fake_bin = fixture / "fake-bin"
            scripts.mkdir()
            frontend.mkdir(parents=True)
            fake_bin.mkdir()
            shutil.copy2(REPO / "scripts/install-build-deps.sh", scripts)
            (fixture / "Cargo.toml").write_text(
                'rust-version = "1.90"\n',
                encoding="utf-8",
            )
            (frontend / "package-lock.json").write_text("{}\n", encoding="utf-8")
            command_log = fixture / "commands.log"

            _write_executable(fake_bin / "rpm", "exit 0\n")
            _write_executable(fake_bin / "cargo", "exit 0\n")
            _write_executable(
                fake_bin / "rustc",
                """
case "${1:-}" in
  --version) echo 'rustc 1.97.1 (fixture)' ;;
  -vV) printf 'rustc 1.97.1 (fixture)\nhost: aarch64-unknown-linux-gnu\n' ;;
esac
""",
            )
            _write_executable(fake_bin / "aarch64-linux-musl-gcc", "exit 0\n")
            _write_executable(fake_bin / "node", "echo 'v22.0.0'\n")
            _write_executable(
                fake_bin / "npm",
                """
printf 'npm %s\n' "$*" >>"$FAKE_COMMAND_LOG"
test "${1:-}" = ci
test "${2:-}" = --prefix
mkdir -p "$3/node_modules"
""",
            )
            _write_executable(
                fake_bin / "dnf",
                """
printf 'dnf %s\n' "$*" >>"$FAKE_COMMAND_LOG"
exit 97
""",
            )
            _write_executable(fake_bin / "sudo", 'exec "$@"\n')

            environment = os.environ.copy()
            environment["PATH"] = f"{fake_bin}:/usr/bin:/bin"
            environment["FAKE_COMMAND_LOG"] = str(command_log)
            completed = subprocess.run(
                [str(scripts / "install-build-deps.sh"), "--install"],
                check=False,
                capture_output=True,
                text=True,
                env=environment,
            )
            commands = (
                command_log.read_text(encoding="utf-8").splitlines()
                if command_log.exists()
                else []
            )

        self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)
        self.assertFalse(
            any(command.startswith("dnf ") for command in commands),
            commands,
        )
        self.assertTrue(
            any(command.startswith("npm ci --prefix ") for command in commands),
            commands,
        )


def _write_executable(path: Path, body: str) -> None:
    path.write_text("#!/usr/bin/env bash\nset -euo pipefail\n" + body, encoding="utf-8")
    path.chmod(0o755)


if __name__ == "__main__":
    unittest.main()
