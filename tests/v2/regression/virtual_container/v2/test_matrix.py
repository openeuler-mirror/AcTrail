from __future__ import annotations

import unittest

from tests.v2.regression.virtual_container.v2.matrix import (
    DATA_CASES,
    INTERFACE_CASES,
)


class VirtualContainerMatrixTest(unittest.TestCase):
    def test_keeps_the_seven_required_acceptance_cells(self) -> None:
        self.assertEqual(
            tuple(case.name for case in INTERFACE_CASES),
            ("verify", "deny", "launch", "namespace"),
        )
        self.assertEqual(
            tuple(case.name for case in DATA_CASES),
            ("tls-only", "ebpf-only", "combo"),
        )
        deny = next(case for case in INTERFACE_CASES if case.name == "deny")
        self.assertTrue(deny.expect_failure)
        self.assertFalse(deny.refreshable_failure)


if __name__ == "__main__":
    unittest.main()
