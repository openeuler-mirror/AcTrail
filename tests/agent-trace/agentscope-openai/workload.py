#!/usr/bin/env python3
"""AgentScope-native agent that calls an OpenAI-compatible chat endpoint."""

from __future__ import annotations

import argparse
import asyncio
import os

from agentscope.agent import Agent
from agentscope.credential import OpenAICredential
from agentscope.message import UserMsg
from agentscope.model import OpenAIChatModel


async def run_agent(args: argparse.Namespace) -> str:
    api_key = os.environ.get(args.api_key_env)
    if not api_key:
        raise RuntimeError(f"missing environment variable {args.api_key_env}")

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
        client_kwargs={"timeout": args.request_timeout_seconds},
    )
    agent = Agent(
        name="actrail-agentscope-agent",
        system_prompt=(
            "You are a minimal AgentScope validation agent. "
            "Follow the user's output instruction exactly."
        ),
        model=model,
    )
    reply = await agent.reply(UserMsg(name="user", content=args.prompt))
    text = reply.get_text_content()
    if not text:
        raise RuntimeError("AgentScope agent produced no text content")
    return text


def main() -> int:
    args = parse_args()
    print(asyncio.run(run_agent(args)))
    return 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prompt", required=True)
    parser.add_argument("--model", required=True)
    parser.add_argument("--api-url", required=True)
    parser.add_argument("--api-key-env", required=True)
    parser.add_argument("--request-timeout-seconds", type=float, required=True)
    parser.add_argument("--max-tokens", type=int, required=True)
    parser.add_argument("--context-size", type=int, required=True)
    return parser.parse_args()


if __name__ == "__main__":
    raise SystemExit(main())
