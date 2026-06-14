"""Shared Java LangChain4j workload helpers for docs examples."""

from __future__ import annotations

import os
import re
import shutil
import subprocess
from dataclasses import dataclass
from pathlib import Path


FAT_JAR_NAME = "java-langchain4j-agent-0.1.0-all.jar"
MIN_JAVA_MAJOR = 17


@dataclass(frozen=True)
class LlmSettings:
    prompt: str
    expected_output_fragment: str
    model: str
    base_url: str
    api_key_env: str
    request_timeout_seconds: str
    prompt_overridden: bool
    expected_overridden: bool


def resolve_llm_settings(values: dict[str, str], required) -> LlmSettings:
    prompt_overridden = "ACTRAIL_LLM_PROMPT" in os.environ
    expected_overridden = "ACTRAIL_LLM_EXPECTED_OUTPUT_FRAGMENT" in os.environ
    prompt = os.environ.get("ACTRAIL_LLM_PROMPT", required(values, "prompt"))
    model = os.environ.get("ACTRAIL_LLM_MODEL", required(values, "model"))
    api_key_env = os.environ.get("ACTRAIL_LLM_API_KEY_ENV", required(values, "api_key_env"))
    expected = os.environ.get(
        "ACTRAIL_LLM_EXPECTED_OUTPUT_FRAGMENT",
        required(values, "expected_output_fragment"),
    )
    return LlmSettings(
        prompt=prompt,
        expected_output_fragment=expected,
        model=model,
        base_url=resolve_langchain4j_base_url(values, required),
        api_key_env=api_key_env,
        request_timeout_seconds=required(values, "request_timeout_seconds"),
        prompt_overridden=prompt_overridden,
        expected_overridden=expected_overridden,
    )


def resolve_langchain4j_base_url(values: dict[str, str], required) -> str:
    base_url = os.environ.get("ACTRAIL_LLM_BASE_URL", required(values, "base_url"))
    chat_path = os.environ.get("ACTRAIL_LLM_CHAT_PATH", required(values, "chat_path"))
    endpoint = join_url(base_url, chat_path)
    suffix = "/chat/completions"
    if not endpoint.endswith(suffix):
        raise RuntimeError(
            "LangChain4j OpenAiChatModel appends /chat/completions; "
            "ACTRAIL_LLM_CHAT_PATH must end with /chat/completions"
        )
    base = endpoint[: -len(suffix)]
    return base or "/"


def require_https_provider(base_url: str) -> None:
    if not base_url.lower().startswith("https://"):
        raise RuntimeError(
            "Java LangChain4j docs examples must use a real HTTPS provider route. "
            "Plain HTTP/socket fallback would avoid the JSSE path under test: "
            f"{base_url}"
        )


def prepare_maven_project(mvn: str, project_dir: Path, timeout_sec: float, run_checked) -> None:
    print(
        "Preparing Java LangChain4j Maven project; first run may download "
        "dependencies from Maven Central.",
        flush=True,
    )
    run_checked(
        [
            mvn,
            "--batch-mode",
            "-f",
            "pom.xml",
            "-DskipTests",
            "package",
        ],
        echo=True,
        timeout=timeout_sec,
        cwd=project_dir,
    )


def require_fat_jar(project_dir: Path) -> Path:
    fat_jar = project_dir / "target" / FAT_JAR_NAME
    if not fat_jar.is_file():
        raise RuntimeError(
            "Maven package did not produce the expected executable jar: "
            f"{fat_jar}"
        )
    return fat_jar


def java_argv(java: str, fat_jar: Path, llm: LlmSettings) -> list[str]:
    expected = ""
    if not llm.prompt_overridden or llm.expected_overridden:
        expected = llm.expected_output_fragment
    return [
        java,
        "-jar",
        str(fat_jar),
        "--prompt",
        llm.prompt,
        "--expected-output-fragment",
        expected,
        "--model",
        llm.model,
        "--base-url",
        llm.base_url,
        "--api-key-env",
        llm.api_key_env,
        "--request-timeout-seconds",
        llm.request_timeout_seconds,
    ]


def require_tool(name: str) -> str:
    path = shutil.which(name)
    if path is None:
        raise RuntimeError(f"{name} is not on PATH")
    return path


def require_java_major(command: list[str], label: str) -> None:
    result = subprocess.run(command, text=True, capture_output=True, check=False)
    output = f"{result.stdout}\n{result.stderr}".strip()
    if result.returncode != 0:
        raise RuntimeError(f"{label} version check failed: {output}")
    major = parse_java_major(output)
    if major is None:
        raise RuntimeError(f"could not parse {label} version from: {first_line(output)}")
    if major < MIN_JAVA_MAJOR:
        raise RuntimeError(
            f"{label} must use Java {MIN_JAVA_MAJOR}+ for this LangChain4j example; "
            f"found {first_line(output)}"
        )


def parse_java_major(output: str) -> int | None:
    version = parse_version_token(output)
    if version is None:
        return None
    if version.startswith("1."):
        parts = version.split(".")
        return int(parts[1]) if len(parts) > 1 and parts[1].isdigit() else None
    match = re.match(r"(\d+)", version)
    return int(match.group(1)) if match else None


def parse_version_token(output: str) -> str | None:
    quoted = re.search(r'version "([^"]+)"', output)
    if quoted:
        return quoted.group(1)
    javac = re.search(r"\bjavac\s+([0-9][^\s]*)", output)
    if javac:
        return javac.group(1)
    maven = re.search(r"\bJava version:\s*([0-9][^,\s]*)", output)
    if maven:
        return maven.group(1)
    return None


def first_line(output: str) -> str:
    for line in output.splitlines():
        stripped = line.strip()
        if stripped:
            return stripped
    return "<empty output>"


def require_workload_answer(output: str, llm: LlmSettings, completion_marker: str) -> None:
    if completion_marker not in output:
        raise RuntimeError("workload did not report completion after the LLM call")
    answer = parse_answer(output)
    if not answer.strip():
        raise RuntimeError("workload returned an empty LLM answer")
    if not llm.prompt_overridden or llm.expected_overridden:
        if llm.expected_output_fragment not in answer and llm.expected_output_fragment not in output:
            raise RuntimeError("LLM answer did not contain the expected marker")


def parse_answer(output: str) -> str:
    for line in output.splitlines():
        if line.startswith("llm_answer="):
            return line.split("=", 1)[1]
    raise RuntimeError("workload output did not include llm_answer")


def join_url(base_url: str, path: str) -> str:
    return base_url.rstrip("/") + "/" + path.lstrip("/")
