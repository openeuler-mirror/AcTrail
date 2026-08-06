"""Shared argparse value parsers for virtual-container host tools."""

from __future__ import annotations

import argparse


def positive_int(value: str) -> int:
    """Parse an integer greater than zero for an argparse option."""
    try:
        parsed = int(value)
    except ValueError as error:
        raise argparse.ArgumentTypeError("value must be an integer") from error
    if parsed < 1:
        raise argparse.ArgumentTypeError("value must be at least 1")
    return parsed
