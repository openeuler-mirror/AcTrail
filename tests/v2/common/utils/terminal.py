from __future__ import annotations

from typing import Literal, TextIO


TerminalColor = Literal["cyan", "green", "yellow"]

_ANSI_COLORS: dict[TerminalColor, str] = {
    "cyan": "\033[1;36m",
    "green": "\033[1;32m",
    "yellow": "\033[1;33m",
}
_ANSI_RESET = "\033[0m"


def colorize(
    text: str,
    color: TerminalColor,
    stream: TextIO,
) -> str:
    if not stream.isatty():
        return text
    return f"{_ANSI_COLORS[color]}{text}{_ANSI_RESET}"
