#!/usr/bin/env python3
"""Exercise the compiled ABI integration with a real Xiaoo TUI and fresh defaults."""

import argparse
import json
import os
from pathlib import Path
import select
import shlex
import shutil
import socket
import subprocess
import tempfile
import time
import uuid


class CheckpointE2E:
    def __init__(self, args):
        self.args = args
        self.name = "actrail-ckpt-" + uuid.uuid4().hex[:10]
        self.driver = None
        self.started = False
        self.export = Path.home() / ".xiaoo/session" / (self.name + ".json")

    def tmux(self, *args):
        return subprocess.run(
            ["tmux", *args], check=True, text=True, capture_output=True
        ).stdout

    def pane(self):
        return self.tmux("capture-pane", "-p", "-t", self.name)

    def wait(self, predicate, label):
        deadline = time.monotonic() + self.args.timeout
        while time.monotonic() < deadline:
            if predicate():
                return
            time.sleep(0.2)
        raise RuntimeError(f"timed out: {label}\n{self.pane()}")

    def prompt(self, message):
        self.tmux("send-keys", "-t", self.name, "-l", "--", message)
        self.tmux("send-keys", "-t", self.name, "Enter")
        # Let the TUI consume Enter before checking its idle footer.
        time.sleep(1)
        self.wait(lambda: "Enter 发送" in self.pane(), "agent turn completion")

    def call(self, op, **fields):
        command = {"op": op, **fields}
        self.driver.stdin.write(json.dumps(command) + "\n")
        self.driver.stdin.flush()
        readable, _, _ = select.select([self.driver.stdout], [], [], 15)
        if not readable:
            raise RuntimeError(f"ABI driver timed out on {op}")
        line = self.driver.stdout.readline()
        if not line:
            raise RuntimeError(f"ABI driver exited during {op}")
        return json.loads(line)

    def success(self, op, **fields):
        response = self.call(op, **fields)
        if not response["ok"]:
            raise RuntimeError(f"{op} failed: {response}")
        print(f"PASS ABI {op}", flush=True)
        return response["result"]

    def direct(self, endpoint, request):
        with socket.socket(socket.AF_UNIX) as stream:
            stream.settimeout(10)
            stream.connect(str(endpoint))
            stream.sendall((json.dumps(request) + "\n").encode())
            with stream.makefile("rb") as reply:
                return json.loads(reply.readline())

    def run(self):
        with tempfile.TemporaryDirectory(prefix=self.name + "-") as directory:
            root = Path(directory)
            workspace = root / "workspace"
            workspace.mkdir()
            endpoint = root / "checkpoint.sock"
            # Keep the default save mode so init must select current over next.
            config = root / "config.toml"
            config.write_text(
                '[llm]\nprovider = "deepseek"\nmodel = "deepseek-flash"\n'
                'api_key_env = "DEEPSEEK_API_KEY"\n'
                '[checkpoint]\nenabled = true\n'
                '[checkpoint.unix_socket]\nendpoint = ' + json.dumps(str(endpoint)) + '\n'
            )
            for name in ("llm_secrets.json", ".llm_secrets.json.key"):
                shutil.copyfile(self.args.secrets_dir / name, root / name)
                os.chmod(root / name, 0o600)
            integration_config = root / "integration.json"
            integration_config.write_text(json.dumps({"endpoint": str(endpoint)}))
            command = shlex.join([str(self.args.xiaoo), "--config", str(config)])
            try:
                self.tmux("new-session", "-d", "-s", self.name, "-x", "160", "-y", "50", "-c", str(workspace), command)
                self.started = True
                self.wait(lambda: endpoint.exists(), "checkpoint socket startup")
                self.wait(lambda: "Enter 发送" in self.pane(), "agent ready")
                self.driver = subprocess.Popen(
                    [str(self.args.driver), str(self.args.library), str(integration_config)],
                    stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1,
                )
                self.success("connect", trace_id=self.name)
                alpha = "ALPHA_" + self.name.replace("-", "_")
                beta = "BETA_" + self.name.replace("-", "_")
                self.prompt(f"记住本会话唯一口令为 {alpha}。只回复这个口令，不调用工具。")
                snapshot = self.success("snapshot")
                self.prompt(f"将本会话唯一口令替换为 {beta}。只回复这个新口令，不调用工具。")
                self.success("restore", snapshot_id=snapshot)
                self.prompt("现在本会话唯一口令是什么？只回复口令，不调用工具。")
                self.prompt("/save " + self.name)
                self.wait(self.export.exists, "session export")
                saved = json.loads(self.export.read_text())
                messages = saved.get("session_messages")
                if messages is None:
                    messages = saved["session_record"]["loop_state"]["messages"]
                transcript = json.dumps(messages, ensure_ascii=False)
                if alpha not in transcript or beta in transcript:
                    raise RuntimeError("restored context does not match the saved ALPHA boundary")
                # Verify a real assistant reply after restoration, not only user input.
                assistant = [m for m in messages if m.get("role") == "assistant"]
                if not assistant or alpha not in json.dumps(assistant[-1], ensure_ascii=False):
                    raise RuntimeError("the real agent did not answer with the restored token")
                print("PASS real agent restored ALPHA context; BETA absent", flush=True)
                self.success("delete", snapshot_id=snapshot)
                rejected = self.call("restore", snapshot_id=snapshot)
                if rejected["ok"]:
                    raise RuntimeError("deleted snapshot was accepted for restore")
                print("PASS deleted snapshot rejected", flush=True)
                self.success("disconnect")
                if self.call("snapshot")["ok"]:
                    raise RuntimeError("disconnected backend accepted snapshot")
                released = self.direct(endpoint, {"op": "save", "context": {"session_id": snapshot["session_id"]}})
                if released["ok"]:
                    raise RuntimeError("Xiaoo release did not close the session gate")
                print("PASS Xiaoo release gate closed", flush=True)
                self.success("connect", trace_id=self.name + "-reconnected")
                self.success("disconnect")
            finally:
                if self.driver is not None:
                    self.driver.stdin.close()
                    try:
                        self.driver.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        self.driver.terminate()
                        self.driver.wait(timeout=5)
                if self.started:
                    subprocess.run(["tmux", "kill-session", "-t", self.name], capture_output=True)
                self.export.unlink(missing_ok=True)


def main():
    root = Path(__file__).resolve().parents[5]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--xiaoo", type=Path, required=True)
    parser.add_argument("--library", type=Path, default=root / "target/release/libactrail_ckpt_cxt_xiaoo.so")
    parser.add_argument("--driver", type=Path, default=root / "target/release/examples/xiaoo_checkpoint_e2e")
    parser.add_argument("--secrets-dir", type=Path, default=Path.home() / ".config/xiaoo")
    parser.add_argument("--timeout", type=float, default=120)
    CheckpointE2E(parser.parse_args()).run()


if __name__ == "__main__":
    main()
