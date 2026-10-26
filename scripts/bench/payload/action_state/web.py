"""Read completed real action-state traces through a separately owned web server."""
import argparse
import json
import subprocess
import time
from pathlib import Path
from urllib.parse import quote
from urllib.request import urlopen
from urllib.error import HTTPError


class WebAcceptance:
    def __init__(self, args):
        self.args = args
        self.request_count = 0

    @staticmethod
    def require(condition, message):
        if not condition:
            raise RuntimeError(message)

    def get(self, path, name):
        try:
            with urlopen(self.base + path, timeout=30) as response:
                document = json.load(response)
        except HTTPError as error:
            body = error.read().decode()
            (self.output / f"{self.request_count:04d}-{name}-error.txt").write_text(body)
            raise RuntimeError(f"{path}: HTTP {error.code}: {body}") from error
        (self.output / f"{self.request_count:04d}-{name}.json").write_text(
            json.dumps(document, indent=2) + "\n")
        self.request_count += 1
        return document

    def compare(self, actual, expected, lite=False):
        self.require(actual["id"] == expected["action_id"], "action ID differs")
        for key in ("kind", "status", "completeness", "title"):
            self.require(actual[key] == expected[key], f"{actual['id']}: {key} differs")
        if lite:
            for key, value in expected["attributes"].items():
                if key.endswith((".action_id", "_action_id")) or key in (
                    "invocation.kind", "command.tool_name", "mcp.execution.status",
                    "llm.tool_result.binding", "finalization.reason",
                ):
                    self.require(actual["attributes"].get(key) == value,
                                 f"{actual['id']}: paged {key} differs")
        else:
            self.require(actual["attributes"] == expected["attributes"],
                         f"{actual['id']}: full attributes differ")
            key = lambda item: (item["kind"], item["id"], item["role"])
            self.require(sorted(actual["evidence"], key=key) == sorted(expected["evidence"], key=key),
                         f"{actual['id']}: evidence differs")

    def roles(self, graph, actions):
        report = {}
        for role, parent_kind, child_kind, attr, on_parent in (
            ("llm.call.http_response", "llm.call", "http.message", "llm.call.http_response_action_id", True),
            ("llm.request.tool_result", "llm.request", "llm.tool_result", "llm.tool_result.request_action_id", False),
        ):
            links = [link for link in graph["links"] if link["valid"] and link["role"] == role]
            for link in links:
                parent, child = actions[link["parent_action_id"]], actions[link["child_action_id"]]
                self.require(parent["kind"] == parent_kind and child["kind"] == child_kind,
                             f"{role}: endpoint kinds differ")
                self.require(parent["process"] == child["process"] and parent["trace_id_raw"] == child["trace_id_raw"],
                             f"{role}: endpoint identities differ")
                owner, target = (parent, child) if on_parent else (child, parent)
                self.require(owner["attributes"].get(attr) == target["action_id"],
                             f"{role}: hydrated reference differs")
            references = [action for action in actions.values() if attr in action["attributes"]]
            self.require(len(links) == len(references), f"{role}: reference/link counts differ")
            report[role] = {"count": len(links), "status": "passed" if links else "not_exercised"}
        return report

    def pages(self, prefix, parent):
        offset, items, total, pages = 0, [], None, 0
        while True:
            page = self.get(prefix + "/action-tree/children/" + quote(parent, safe="")
                            + f"?offset={offset}&limit=1", "children")
            self.require(page["offset"] == offset and page["limit"] == 1, "pagination window differs")
            if total is None:
                total = page["total"]
            self.require(page["total"] == total, "completed tree changed during pagination")
            items.extend(page["actions"])
            pages += 1
            if not page["has_more"]:
                self.require(page["next_offset"] is None, "terminal page has next offset")
                break
            self.require(page["next_offset"] > offset, "pagination did not advance")
            offset = page["next_offset"]
        self.require(len(items) == total and len({item['id'] for item in items}) == total,
                     f"{parent}: missing or duplicated paged children")
        return items, pages

    def mode(self, mode):
        self.output = self.args.out / mode
        self.output.mkdir()
        result = json.loads((self.args.run_dir / mode / "result.json").read_text())
        self.require(result["status"] == "passed", "source MCP acceptance did not pass")
        trace = result["finalization"]["traces"][0][0]
        graph = json.loads((self.args.run_dir / mode / "actions.json").read_text())
        expected = {action["action_id"]: action for action in graph["actions"]}
        role_report = self.roles(graph, expected)
        (self.output / "roles.json").write_text(json.dumps(role_report, indent=2) + "\n")
        log_path = self.output / "web.log"
        with log_path.open("w") as log:
            process = subprocess.Popen([str(self.args.bin_dir / "actrailweb"), "--config",
                result["resolved_config"], "--addr", "127.0.0.1", "--port", "0"],
                stdout=log, stderr=subprocess.STDOUT)
            try:
                deadline = time.monotonic() + 20
                self.base = None
                while time.monotonic() < deadline:
                    self.require(process.poll() is None, "owned web process exited at startup")
                    for line in log_path.read_text().splitlines():
                        if line.startswith("actrailweb listening on "):
                            self.base = line.removeprefix("actrailweb listening on ").split()[0]
                            break
                    if self.base:
                        break
                    time.sleep(0.05)
                self.require(self.base, "web did not report listening URL")
                prefix = f"/api/traces/{trace}"
                root = self.get(prefix + "/action-tree/root", "root")
                observed = root["root"]["observed_agent"]
                if observed is not None:
                    # The root query decorates its process.exec with observed agent identity.
                    self.compare(observed, expected[observed["id"]], lite=True)
                    for key, value in expected[observed["id"]]["attributes"].items():
                        self.require(observed["attributes"].get(key) == value, f"observed root lost {key}")
                    evidence_id = observed["attributes"].get("agent.identity.evidence_action_id")
                    self.require(evidence_id in expected, "observed root identity evidence target missing")
                for action in expected.values():
                    detail = self.get(prefix + "/actions/" + quote(action["action_id"], safe=""), "detail")
                    self.compare(detail, action)
                queue, visited, found, page_count = [root["root"]["id"]], set(), set(), 0
                while queue:
                    parent = queue.pop(0)
                    if parent in visited:
                        continue
                    visited.add(parent)
                    children, pages = self.pages(prefix, parent)
                    page_count += pages
                    for child in children:
                        self.compare(child, expected[child["id"]], lite=True)
                        found.add(child["id"])
                        queue.append(child["id"])
                required = {key for key, action in expected.items()
                            if action["kind"].startswith("mcp.") or action["kind"] in ("llm.call", "llm.response", "command.invocation")}
                self.require(required <= found, f"display tree missing required actions: {sorted(required - found)}")
                return {"status": "passed", "trace_id": trace, "detail_actions": len(expected),
                        "tree_actions": len(found), "paged_parents": len(visited), "pages": page_count,
                        "web_pid": process.pid, "source": str(self.args.run_dir / mode), "roles": role_report,
                        "not_exercised": ["late-arrival lifecycle", "damaged HTTP response", "concurrent live writes"]}
            finally:
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=5)

    def run(self):
        self.args.out.mkdir(parents=True, exist_ok=False)
        results = {}
        try:
            for mode in self.args.modes:
                try:
                    results[mode] = self.mode(mode)
                except Exception as error:
                    results[mode] = {"status": "failed", "error": str(error)}
                    raise
        finally:
            (self.args.out / "acceptance.json").write_text(json.dumps(results, indent=2) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run-dir", type=Path, required=True)
    parser.add_argument("--bin-dir", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--modes", nargs="+", default=["P", "C"], choices=["P", "C"])
    args = parser.parse_args()
    args.run_dir, args.bin_dir, args.out = args.run_dir.resolve(), args.bin_dir.resolve(), args.out.resolve()
    WebAcceptance(args).run()


if __name__ == "__main__":
    main()
