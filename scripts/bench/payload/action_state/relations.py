"""Verify relationship identities in exported real-agent action graphs."""
import argparse
import json
from pathlib import Path

from .web import WebAcceptance


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--graphs", type=Path, nargs="+", required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    verifier = WebAcceptance(None)
    reports = {}
    for path in args.graphs:
        graph = json.loads(path.read_text())
        actions = {action["action_id"]: action for action in graph["actions"]}
        reports[str(path.resolve())] = verifier.roles(graph, actions)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(reports, indent=2) + "\n")


if __name__ == "__main__":
    main()
