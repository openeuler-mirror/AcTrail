from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path


REPO = Path(__file__).resolve().parents[5]
CONFIGURATOR = REPO / "deploy/virtual-container/host/prepare-stratovirt-config.py"


class VmmConfigPreparerTest(unittest.TestCase):
    def test_generates_each_supported_backend_section(self) -> None:
        cases = (
            ("stratovirt", "stratovirt"),
            ("cloud-hypervisor", "clh"),
        )
        for backend, section in cases:
            with self.subTest(backend=backend), tempfile.TemporaryDirectory(
                prefix="actrail-vmm-config."
            ) as raw_dir:
                root = Path(raw_dir)
                prefix = root / "kata"
                prefix.mkdir()
                (prefix / "VERSION").write_text("3.32.0\n", encoding="utf-8")
                source = root / "source.toml"
                source.write_text(_source_config(section), encoding="utf-8")
                hypervisor = _executable(root / backend)
                virtiofsd = _executable(root / "virtiofsd")
                kernel = _file(root / "kernel")
                image = _file(root / "image")
                published_image = root / "published/image"
                output = root / "candidate.toml"

                result = subprocess.run(
                    [
                        "python3",
                        str(CONFIGURATOR),
                        "--backend",
                        backend,
                        "--kata-prefix",
                        str(prefix),
                        "--base-config",
                        str(source),
                        "--output",
                        str(output),
                        "--hypervisor",
                        str(hypervisor),
                        "--kernel",
                        str(kernel),
                        "--image",
                        str(image),
                        "--image-config-path",
                        str(published_image),
                        "--virtiofsd",
                        str(virtiofsd),
                        "--default-vcpus",
                        "2",
                        "--debug",
                    ],
                    text=True,
                    capture_output=True,
                    check=False,
                )

                self.assertEqual(result.returncode, 0, result.stderr)
                generated = output.read_text(encoding="utf-8")
                self.assertIn(f"[hypervisor.{section}]", generated)
                self.assertIn(f'path = "{hypervisor}"', generated)
                self.assertIn(f'kernel = "{kernel}"', generated)
                self.assertIn(f'image = "{published_image}"', generated)
                self.assertIn(f'valid_hypervisor_paths = ["{hypervisor}"]', generated)
                self.assertIn(f'virtio_fs_daemon = "{virtiofsd}"', generated)
                self.assertIn("default_vcpus = 2", generated)
                self.assertIn("debug_console_enabled = true", generated)
                self.assertIn(f"backend={backend}", result.stdout)


def _source_config(section: str) -> str:
    return (
        f"[hypervisor.{section}]\n"
        'path = "/old/vmm"\n'
        'kernel = "/old/kernel"\n'
        'image = "/old/image"\n'
        'valid_hypervisor_paths = ["/old/vmm"]\n'
        'virtio_fs_daemon = "/old/virtiofsd"\n'
        'valid_virtio_fs_daemon_paths = ["/old/virtiofsd"]\n'
        "default_vcpus = 1\n"
        "enable_debug = false\n"
        "[agent.kata]\n"
        "enable_debug = false\n"
        "debug_console_enabled = false\n"
        "[runtime]\n"
        "enable_debug = false\n"
    )


def _file(path: Path) -> Path:
    path.write_bytes(path.name.encode())
    return path


def _executable(path: Path) -> Path:
    _file(path)
    path.chmod(0o755)
    return path


if __name__ == "__main__":
    unittest.main()
