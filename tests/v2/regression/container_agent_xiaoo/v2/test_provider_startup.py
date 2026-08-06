from __future__ import annotations

import unittest
from pathlib import Path
from urllib.parse import urlsplit

from tests.v2.regression.container_agent_xiaoo.v2.xiaoo_scenario import (
    start_provider,
)


REPO = Path(__file__).resolve().parents[5]
PROVIDER_SCRIPT = REPO / "tests/support/llm-http-proxy/provider_proxy.py"


class ContainerXiaooProviderStartupTest(unittest.TestCase):
    def test_host_network_provider_listens_on_loopback(self) -> None:
        process, listen_url = start_provider(
            PROVIDER_SCRIPT,
            "ACTRAIL_CONTAINER_XIAOO_PROVIDER_OK",
            5,
            REPO,
        )
        try:
            self.assertEqual(urlsplit(listen_url).hostname, "127.0.0.1")
        finally:
            process.terminate()
            process.communicate(timeout=5)


if __name__ == "__main__":
    unittest.main()
