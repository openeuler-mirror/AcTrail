from __future__ import annotations

from pathlib import Path

from tests.v2.common.kata_runtime import sha256_file


class CloudHypervisorAssetBundle:
    @staticmethod
    def xiaoo_config() -> str:
        return """[llm]
provider = "deepseek"
model = "deepseek-chat"
api_key_env = "ACTRAIL_VIRTUAL_XIAOO_API_KEY"
api_base = "http://127.0.0.1:18098"
max_tokens = 128
context_window = 32768
reasoning_effort = "off"
"""

    @staticmethod
    def write_manifest(directory: Path) -> None:
        lines = []
        for path in sorted(directory.iterdir()):
            if path.name == "MANIFEST.sha256" or not path.is_file():
                continue
            lines.append(f"{sha256_file(path)}  ./{path.name}\n")
        (directory / "MANIFEST.sha256").write_text(
            "".join(lines),
            encoding="utf-8",
        )
