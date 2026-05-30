#!/usr/bin/env python3
"""A real LangGraph-based Python agent that calls an OpenAI-compatible API."""

from __future__ import annotations

import argparse
import os
from typing import TypedDict

import requests
from langgraph.graph import END, StateGraph


class AgentState(TypedDict):
    prompt: str
    answer: str


def main() -> int:
    args = parse_args()
    api_key = os.environ.get(args.api_key_env)
    if not api_key:
        raise RuntimeError(f"missing environment variable {args.api_key_env}")

    def call_model(state: AgentState) -> AgentState:
        response = requests.post(
            args.api_url,
            headers={
                "Authorization": f"Bearer {api_key}",
                "Content-Type": "application/json",
            },
            json={
                "model": args.model,
                "messages": [{"role": "user", "content": state["prompt"]}],
                "stream": False,
                "temperature": 0,
            },
            timeout=args.request_timeout_seconds,
        )
        response.raise_for_status()
        data = response.json()
        answer = data["choices"][0]["message"]["content"]
        return {"prompt": state["prompt"], "answer": answer}

    graph = StateGraph(AgentState)
    graph.add_node("call_model", call_model)
    graph.set_entry_point("call_model")
    graph.add_edge("call_model", END)
    result = graph.compile().invoke({"prompt": args.prompt, "answer": ""})
    print(result["answer"])
    return 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prompt", required=True)
    parser.add_argument("--model", required=True)
    parser.add_argument("--api-url", required=True)
    parser.add_argument("--api-key-env", required=True)
    parser.add_argument("--request-timeout-seconds", type=float, default=90)
    return parser.parse_args()


if __name__ == "__main__":
    raise SystemExit(main())
