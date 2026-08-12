"""Structured help messages shared by startup logging and the /help endpoint."""

from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class HelpSection:
    title: str
    lines: tuple[str, ...] = ()


@dataclass(frozen=True, slots=True)
class HelpMessage:
    title: str
    sections: tuple[HelpSection, ...] = ()

    def render(self, origin: str = "") -> str:
        blocks = [self.title]
        for title, lines in self.iter_sections(origin):
            blocks.append("")
            blocks.append(title)
            blocks.extend(f"  {line}" for line in lines)
        return "\n".join(blocks) + "\n"

    def iter_sections(
        self,
        origin: str = "",
    ) -> tuple[tuple[str, tuple[str, ...]], ...]:
        return tuple(
            (
                section.title,
                tuple(
                    line.replace("{origin}", origin)
                    for line in section.lines
                ),
            )
            for section in self.sections
        )


class HelpMessageMixin:
    """Attach a rendered help message to any server application."""

    def set_help_message(self, message: HelpMessage) -> None:
        self._help_message = message

    def help_text(self, origin: str) -> str:
        message = getattr(self, "_help_message", None)
        if message is None:
            return ""
        return message.render(origin)
