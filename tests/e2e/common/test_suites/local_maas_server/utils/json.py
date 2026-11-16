from __future__ import annotations

import json
import math
from typing import Any


class StrictJsonError(ValueError):
    pass


class StrictJsonDecoder:
    def decode_utf8(self, payload: bytes) -> Any:
        try:
            text = payload.decode("utf-8")
        except UnicodeDecodeError as error:
            raise StrictJsonError("JSON must be UTF-8") from error
        try:
            value = json.loads(
                text,
                object_pairs_hook=self._object_from_pairs,
                parse_constant=self._reject_constant,
            )
        except json.JSONDecodeError as error:
            raise StrictJsonError(
                f"invalid JSON at line {error.lineno}, column {error.colno}: "
                f"{error.msg}"
            ) from error
        self._validate(value)
        return value

    def _validate(self, value: Any) -> None:
        if isinstance(value, str):
            try:
                value.encode("utf-8")
            except UnicodeEncodeError as error:
                raise StrictJsonError(
                    "JSON strings must contain valid Unicode scalar values"
                ) from error
            return
        if isinstance(value, float):
            if not math.isfinite(value):
                raise StrictJsonError("JSON numbers must be finite")
            return
        if isinstance(value, list):
            for item in value:
                self._validate(item)
            return
        if isinstance(value, dict):
            for key, item in value.items():
                self._validate(key)
                self._validate(item)

    @staticmethod
    def _object_from_pairs(
        pairs: list[tuple[str, Any]],
    ) -> dict[str, Any]:
        result: dict[str, Any] = {}
        for key, value in pairs:
            if key in result:
                raise StrictJsonError(f"duplicate JSON object key: {key!r}")
            result[key] = value
        return result

    @staticmethod
    def _reject_constant(value: str) -> None:
        raise StrictJsonError(f"invalid JSON number constant: {value}")
