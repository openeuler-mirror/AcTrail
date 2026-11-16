"""Drive a real Xiaoo TUI and inspect its exported conversation."""

from __future__ import annotations

import json
import os
import shlex
import shutil
import subprocess
import time
import uuid
from pathlib import Path


class XiaooSession:
    def __init__(self, root: Path):
        self.name = "actrail-composite-" + uuid.uuid4().hex[:10]
        self.root = root
        self.endpoint = root / "xiaoo.sock"
        self.config = root / "xiaoo.toml"
        self.binary = Path(os.environ.get("XIAOO_BINARY", shutil.which("xiaoo") or "xiaoo"))
        self.secrets = Path(os.environ.get("XIAOO_SECRETS_DIR", Path.home() / ".config/xiaoo"))
        self.timeout = float(os.environ.get("ACTRAIL_COMPOSITE_TIMEOUT_SECONDS", "180"))
        self.export = Path.home() / ".xiaoo/session" / (self.name + ".json")
        self.started = False

    def missing(self) -> str | None:
        if not shutil.which("tmux"):
            return "tmux is required for the real Xiaoo TUI"
        if not self.binary.is_file():
            return f"Xiaoo binary not found: {self.binary}"
        for name in ("llm_secrets.json", ".llm_secrets.json.key"):
            if not (self.secrets / name).is_file():
                return f"Xiaoo credential file not found: {self.secrets / name}"
        return None

    def prepare(self) -> None:
        # A minimal fresh configuration uses Xiaoo defaults for all other fields.
        self.root.mkdir(parents=True, exist_ok=True)
        self.config.write_text(
            '[llm]\nprovider = "deepseek"\nmodel = "deepseek-flash"\n'
            'api_key_env = "DEEPSEEK_API_KEY"\n'
            '[checkpoint]\nenabled = true\n'
            '[checkpoint.unix_socket]\nendpoint = ' + json.dumps(str(self.endpoint)) + '\n',
            encoding="utf-8",
        )
        for name in ("llm_secrets.json", ".llm_secrets.json.key"):
            destination = self.root / name
            shutil.copyfile(self.secrets / name, destination)
            destination.chmod(0o600)

    def tmux(self, *args: str) -> str:
        return subprocess.run(
            ["tmux", *args], check=True, text=True, capture_output=True, timeout=15
        ).stdout

    def pane(self) -> str:
        return self.tmux("capture-pane", "-p", "-t", self.name)

    def wait(self, predicate, label: str) -> None:
        deadline = time.monotonic() + self.timeout
        while time.monotonic() < deadline:
            if self.tmux("display-message", "-p", "-t", self.name, "#{pane_dead}").strip() == "1":
                raise AssertionError(f"Xiaoo exited while waiting for {label}\n{self.pane()}")
            if predicate():
                return
            time.sleep(0.2)
        raise AssertionError(f"timed out waiting for {label}\n{self.pane()}")

    def start(self, launch: list[str], workspace: Path) -> None:
        command = shlex.join([*launch, str(self.binary), "--config", str(self.config)])
        # Keep launch errors visible even when the wrapper exits before Xiaoo starts.
        self.tmux("new-session", "-d", "-s", self.name, "-x", "160", "-y", "50",
                  "-c", str(workspace))
        self.started = True
        self.tmux("set-option", "-t", self.name, "remain-on-exit", "on")
        self.tmux("send-keys", "-t", self.name, "-l", "--", "exec " + command)
        self.tmux("send-keys", "-t", self.name, "Enter")
        self.wait(self.endpoint.exists, "Xiaoo checkpoint socket after launch")
        self.wait(lambda: "Enter 发送" in self.pane(), "Xiaoo input prompt")

    def send(self, message: str) -> None:
        self.tmux("send-keys", "-t", self.name, "-l", "--", message)
        self.tmux("send-keys", "-t", self.name, "Enter")
        # Let the TUI consume Enter before looking for its idle footer.
        time.sleep(1)

    def finish_turn(self) -> None:
        self.wait(lambda: "Enter 发送" in self.pane(), "real agent turn completion")

    def prompt(self, message: str) -> None:
        self.send(message)
        self.finish_turn()

    def messages(self) -> list[dict]:
        self.export.unlink(missing_ok=True)
        self.prompt("/save " + self.name)
        self.wait(self.export.exists, "Xiaoo conversation export")
        saved = json.loads(self.export.read_text(encoding="utf-8"))
        messages = saved.get("session_messages")
        if messages is None:
            messages = saved["session_record"]["loop_state"]["messages"]
        return messages

    def close(self) -> None:
        if self.started:
            subprocess.run(["tmux", "kill-session", "-t", self.name],
                           capture_output=True, timeout=15)
        self.export.unlink(missing_ok=True)
        for name in ("llm_secrets.json", ".llm_secrets.json.key"):
            (self.root / name).unlink(missing_ok=True)
