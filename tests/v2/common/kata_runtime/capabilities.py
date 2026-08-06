from __future__ import annotations

import re
from dataclasses import dataclass

from .process import CommandRunner


@dataclass(frozen=True)
class CtrCapabilities:
    run_user: bool
    exec_user: bool
    runtime_config_path: bool

    @classmethod
    def detect(
        cls,
        runner: CommandRunner,
        *,
        ctr: str = "ctr",
        timeout: float = 10,
    ) -> CtrCapabilities:
        run_help = runner.run([ctr, "run", "--help"], timeout=timeout)
        if run_help.returncode != 0:
            raise RuntimeError(
                "failed to inspect ctr run capabilities: "
                + (run_help.diagnostic or "no diagnostic output")
            )
        exec_help = runner.run(
            [ctr, "tasks", "exec", "--help"],
            timeout=timeout,
        )
        if exec_help.returncode != 0:
            raise RuntimeError(
                "failed to inspect ctr tasks exec capabilities: "
                + (exec_help.diagnostic or "no diagnostic output")
            )
        return cls(
            run_user=_has_option(run_help.stdout, "--user"),
            exec_user=_has_option(exec_help.stdout, "--user"),
            runtime_config_path=_has_option(
                run_help.stdout,
                "--runtime-config-path",
            ),
        )


def _has_option(help_text: str, option: str) -> bool:
    return re.search(
        rf"(?:^|\s){re.escape(option)}(?:[=,\s]|$)",
        help_text,
        flags=re.MULTILINE,
    ) is not None
