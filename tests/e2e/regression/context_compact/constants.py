"""Constants for the context compaction regression."""
from dataclasses import dataclass


@dataclass(frozen=True)
class Defaults:
    seed_prompt: str = (
        "We are planning a small library catalog with books, borrowers, loans "
        "and returns. Describe its data model and five invariants in about "
        "400 words. Do not use tools."
    )
    followup_prompt: str = (
        "Extend that plan with a concrete loan and return example and failure "
        "handling, in about 300 words. Do not use tools."
    )
    post_compaction_prompt: str = (
        "Explain how to design an inventory system for a small grocery store "
        "in about 150 words. Do not use tools."
    )


DEFAULTS = Defaults()
