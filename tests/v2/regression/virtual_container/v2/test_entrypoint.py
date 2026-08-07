from __future__ import annotations

import unittest

from tests.v2.regression.virtual_container.run_e2e import (
    TEST_DEFINITION as ROOT_DEFINITION,
)
from tests.v2.regression.virtual_container.v2.run_e2e import (
    TEST_DEFINITION as V2_DEFINITION,
)


class VirtualContainerEntrypointTest(unittest.TestCase):
    def test_root_entrypoint_forwards_to_current_v2_definition(self) -> None:
        self.assertIs(ROOT_DEFINITION, V2_DEFINITION)


if __name__ == "__main__":
    unittest.main()
