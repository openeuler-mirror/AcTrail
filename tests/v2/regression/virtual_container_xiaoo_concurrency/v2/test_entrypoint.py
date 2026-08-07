from __future__ import annotations

import unittest

from tests.v2.regression.virtual_container_xiaoo_concurrency.run_e2e import (
    TEST_DEFINITION as ROOT_DEFINITION,
)
from tests.v2.regression.virtual_container_xiaoo_concurrency.v2.run_e2e import (
    TEST_DEFINITION as V2_DEFINITION,
)
from tests.v2.regression.virtual_container_xiaoo_concurrency.v2.scenario import (
    DUAL_VM_INSTANCES,
)


class VirtualContainerXiaooConcurrencyEntrypointTest(unittest.TestCase):
    def test_root_entrypoint_forwards_to_dual_vm_v2(self) -> None:
        self.assertIs(ROOT_DEFINITION, V2_DEFINITION)
        self.assertEqual(DUAL_VM_INSTANCES, ("a", "b"))


if __name__ == "__main__":
    unittest.main()
