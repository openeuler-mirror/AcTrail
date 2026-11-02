from __future__ import annotations

from pathlib import Path

from .constants import CASE_CONSTANTS

from tests.v2.common.kata_runtime import sha256_file


class ExecutionIsolationAssetBundle:
    @staticmethod
    def xiaoo_config() -> str:
        # No api_key_env: since xiaoO a26ee26d it makes xiaoO save the key next
        # to this read-only config and exit. workload.sh passes the key through
        # the provider default variable (DEEPSEEK_API_KEY) instead.
        return f"""[llm]
provider = "{CASE_CONSTANTS.llm_provider}"
model = "{CASE_CONSTANTS.llm_model}"
api_base = "http://{CASE_CONSTANTS.provider_host}:{CASE_CONSTANTS.provider_port}"
max_tokens = {CASE_CONSTANTS.llm_max_tokens}
context_window = {CASE_CONSTANTS.llm_context_window}
reasoning_effort = "{CASE_CONSTANTS.llm_reasoning_effort}"
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
