from __future__ import annotations

import unittest

from tests.v2.common.kata_runtime.backend import (
    kata_backend,
    shared_filesystem_backends,
    supported_backends,
)


class KataBackendTest(unittest.TestCase):
    def test_firecracker_is_a_supported_kata_backend(self) -> None:
        backend = kata_backend("firecracker")

        self.assertIn("firecracker", supported_backends())
        self.assertEqual(backend.vmm_command, "firecracker")
        self.assertEqual(backend.toml_section, "hypervisor.firecracker")
        self.assertEqual(backend.default_config_name, "configuration-fc.toml")
        self.assertFalse(backend.supports_shared_filesystem)
        self.assertEqual(backend.default_snapshotter, "devmapper")

    def test_host_bind_mount_backends_exclude_firecracker(self) -> None:
        self.assertEqual(
            shared_filesystem_backends(),
            ("stratovirt", "cloud-hypervisor"),
        )


if __name__ == "__main__":
    unittest.main()
