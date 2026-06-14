#!/usr/bin/env python3
"""A real LangGraph agent node that calls an OpenAI-compatible chat model."""

from __future__ import annotations

import argparse
import json
import os
from typing import TypedDict

from langchain_openai import ChatOpenAI
from langgraph.graph import END, StateGraph


class AgentState(TypedDict):
    prompt: str
    answer: str


def main() -> int:
    args = parse_args()
    api_key = os.environ.get(args.api_key_env)
    if not api_key:
        raise RuntimeError(f"missing environment variable {args.api_key_env}")

    chat = ChatOpenAI(
        model=args.model,
        api_key=api_key,
        base_url=args.base_url,
        temperature=0,
        timeout=args.request_timeout_seconds,
        max_retries=0,
    )

    def call_model(state: AgentState) -> AgentState:
        response = chat.invoke(state["prompt"])
        answer = message_content_to_text(response.content)
        return {"prompt": state["prompt"], "answer": answer}

    graph = StateGraph(AgentState)
    graph.add_node("call_model", call_model)
    graph.set_entry_point("call_model")
    graph.add_edge("call_model", END)

    result = graph.compile().invoke({"prompt": args.prompt, "answer": ""})
    answer = result["answer"]
    print("llm_answer_json=" + json.dumps(answer, ensure_ascii=False), flush=True)
    print("ACTRAIL_LANGGRAPH_AGENT_COMPLETE", flush=True)
    return 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prompt", required=True)
    parser.add_argument("--model", required=True)
    parser.add_argument("--base-url", required=True)
    parser.add_argument("--api-key-env", required=True)
    parser.add_argument("--request-timeout-seconds", type=float, default=90.0)
    return parser.parse_args()


def message_content_to_text(content: object) -> str:
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        fragments: list[str] = []
        for item in content:
            if isinstance(item, str):
                fragments.append(item)
            elif isinstance(item, dict):
                text = item.get("text")
                if isinstance(text, str):
                    fragments.append(text)
                else:
                    fragments.append(json.dumps(item, ensure_ascii=False))
            else:
                fragments.append(str(item))
        return "".join(fragments)
    return str(content)


if __name__ == "__main__":
    raise SystemExit(main())
