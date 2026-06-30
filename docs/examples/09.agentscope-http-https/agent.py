#!/usr/bin/env python3
"""Native AgentScope agent for AcTrail HTTP/HTTPS observation examples."""

from __future__ import annotations

import argparse
import asyncio
import os
from pathlib import Path

import httpx
from agentscope.agent import Agent, ContextConfig, ReActConfig
from agentscope.credential import OpenAICredential
from agentscope.event import EventType
from agentscope.message import UserMsg
from agentscope.model import OpenAIChatModel
from agentscope.permission import (
    AdditionalWorkingDirectory,
    PermissionContext,
    PermissionMode,
)
from agentscope.state import AgentState
from agentscope.tool import Bash, Edit, Glob, Grep, Read, Toolkit, Write


async def run_agent(args: argparse.Namespace) -> str:
    api_key = os.environ.get(args.api_key_env)
    if not api_key:
        raise RuntimeError(f"missing environment variable {args.api_key_env}")

    tool_workspace = Path(args.tool_workspace).resolve()
    tool_workspace.mkdir(parents=True, exist_ok=True)
    http_client = httpx.AsyncClient(
        timeout=args.request_timeout_seconds,
    )
    try:
        model = OpenAIChatModel(
            credential=OpenAICredential(api_key=api_key, base_url=args.api_url),
            model=args.model,
            parameters=OpenAIChatModel.Parameters(
                max_tokens=args.max_tokens,
                temperature=0,
            ),
            stream=True,
            max_retries=0,
            context_size=args.context_size,
            client_kwargs={"http_client": http_client},
        )
        toolkit = build_toolkit(tool_workspace, args.disable_tools)
        agent = Agent(
            name="actrail-agentscope-agent",
            system_prompt=(
                "You are an AgentScope validation agent for AcTrail. "
                "Use the available tools when they help inspect the local "
                "validation workspace or produce auditable evidence. "
                f"Keep file operations under this workspace: {tool_workspace}. "
                "When the user asks you to run one exact Bash command, call "
                "the Bash tool once with that command. After receiving the "
                "tool result, stop calling tools and produce the final answer. "
                "For exact-output validation prompts, return the requested "
                "text exactly after any required tool work."
            ),
            model=model,
            toolkit=toolkit,
            state=build_agent_state(tool_workspace),
            context_config=ContextConfig(tool_result_limit=args.tool_result_limit),
            react_config=ReActConfig(max_iters=args.max_iters),
        )
        return await run_reply_stream(agent, args.prompt)
    finally:
        await http_client.aclose()


def build_toolkit(tool_workspace: Path, disable_tools: bool) -> Toolkit | None:
    if disable_tools:
        return None
    return Toolkit(
        tools=[
            Bash(cwd=tool_workspace),
            Glob(),
            Grep(),
            Read(),
            Write(),
            Edit(),
        ],
    )


def build_agent_state(tool_workspace: Path) -> AgentState:
    workspace = str(tool_workspace)
    return AgentState(
        permission_context=PermissionContext(
            mode=PermissionMode.ACCEPT_EDITS,
            working_directories={
                workspace: AdditionalWorkingDirectory(
                    path=workspace,
                    source="actrail-agentscope-example",
                ),
            },
        ),
    )


async def run_reply_stream(agent: Agent, prompt: str) -> str:
    text_parts: list[str] = []
    async for event in agent.reply_stream(UserMsg(name="user", content=prompt)):
        print(render_event(event), flush=True)
        if event.type == EventType.TEXT_BLOCK_DELTA:
            text_parts.append(event.delta)
    text = "".join(text_parts).strip()
    if not text:
        raise RuntimeError("AgentScope agent produced no text content")
    return text


def render_event(event) -> str:
    if event.type == EventType.MODEL_CALL_START:
        return f"agentscope_event=model_call_start model={event.model_name}"
    if event.type == EventType.MODEL_CALL_END:
        return (
            "agentscope_event=model_call_end "
            f"input_tokens={event.input_tokens} output_tokens={event.output_tokens}"
        )
    if event.type == EventType.TEXT_BLOCK_DELTA:
        return f"agentscope_text_delta={event.delta}"
    if event.type == EventType.TOOL_CALL_START:
        return (
            "agentscope_event=tool_call_start "
            f"tool_call_id={event.tool_call_id} tool={event.tool_call_name}"
        )
    if event.type == EventType.TOOL_CALL_DELTA:
        return (
            "agentscope_event=tool_call_delta "
            f"tool_call_id={event.tool_call_id} delta={event.delta}"
        )
    if event.type == EventType.TOOL_CALL_END:
        return f"agentscope_event=tool_call_end tool_call_id={event.tool_call_id}"
    if event.type == EventType.TOOL_RESULT_START:
        return f"agentscope_event=tool_result_start tool_call_id={event.tool_call_id}"
    if event.type == EventType.TOOL_RESULT_TEXT_DELTA:
        return (
            "agentscope_event=tool_result_text_delta "
            f"tool_call_id={event.tool_call_id} delta={event.delta}"
        )
    if event.type == EventType.TOOL_RESULT_END:
        return f"agentscope_event=tool_result_end tool_call_id={event.tool_call_id}"
    return f"agentscope_event={event.type.value.lower()}"


def main() -> int:
    args = parse_args()
    text = asyncio.run(run_agent(args))
    print(f"agentscope_final_text={text}")
    return 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prompt", required=True)
    parser.add_argument("--model", required=True)
    parser.add_argument("--api-url", required=True)
    parser.add_argument("--api-key-env", required=True)
    parser.add_argument("--request-timeout-seconds", type=float, default=60)
    parser.add_argument("--max-tokens", type=int, default=512)
    parser.add_argument("--context-size", type=int, default=32768)
    parser.add_argument("--tool-result-limit", type=int, default=4096)
    parser.add_argument("--max-iters", type=int, default=4)
    parser.add_argument(
        "--tool-workspace",
        default="target/docs-examples/agentscope-http-https/tool-workspace",
    )
    parser.add_argument("--disable-tools", action="store_true")
    return parser.parse_args()


if __name__ == "__main__":
    raise SystemExit(main())
