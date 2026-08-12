from __future__ import annotations

import secrets
import uuid
from dataclasses import dataclass
from pathlib import Path
from threading import RLock
from typing import Any, Iterable

from transport.upstream import UpstreamConfig

from .parser import RecordedResponse
from .store import FinalizeResult, RecordStore


class RecordSessionError(RuntimeError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(slots=True)
class RecordSession:
    session_id: str
    api_key: str
    tools: frozenset[str]
    upstream: UpstreamConfig
    cache_path: Path
    state: str = "open"
    response_count: int = 0


class RecordSessionManager:
    def __init__(self, store: RecordStore):
        self._store = store
        self._lock = RLock()
        self._by_id: dict[str, RecordSession] = {}
        self._by_api_key: dict[str, RecordSession] = {}

    def create(
        self,
        tools: Iterable[str],
        upstream: UpstreamConfig,
    ) -> RecordSession:
        tool_names = tuple(tools)
        if not tool_names or any(
            not isinstance(name, str) or not name
            for name in tool_names
        ):
            raise ValueError("tools must be a non-empty list of names")
        session_id = uuid.uuid4().hex[:12]
        api_key = secrets.token_urlsafe(32)
        cache_path = self._store.create_cache(session_id)
        session = RecordSession(
            session_id=session_id,
            api_key=api_key,
            tools=frozenset(name.casefold() for name in tool_names),
            upstream=upstream,
            cache_path=cache_path,
        )
        with self._lock:
            self._by_id[session_id] = session
            self._by_api_key[api_key] = session
        return session

    def get_by_api_key(self, api_key: str) -> RecordSession | None:
        with self._lock:
            return self._by_api_key.get(api_key)

    def get_by_id(self, session_id: str) -> RecordSession | None:
        with self._lock:
            return self._by_id.get(session_id)

    def list_sessions(self) -> tuple[RecordSession, ...]:
        with self._lock:
            return tuple(self._by_id.values())

    def append(
        self,
        session: RecordSession,
        recorded: RecordedResponse,
    ) -> None:
        with self._lock:
            if session.state != "open":
                raise RecordSessionError(
                    "session_not_open",
                    f"recording session {session.session_id} is not open",
                )
            self._store.append(session.cache_path, recorded)
            session.response_count += 1

    def finalize(
        self,
        session_id: str,
        scenario_id: str,
    ) -> FinalizeResult:
        with self._lock:
            session = self._by_id.get(session_id)
            if session is None:
                raise RecordSessionError(
                    "session_not_found",
                    f"recording session {session_id} does not exist",
                )
            if session.state != "open":
                raise RecordSessionError(
                    "session_not_open",
                    f"recording session {session_id} is already finalized",
                )
            result = self._store.finalize(
                session.session_id,
                session.cache_path,
                scenario_id,
            )
            session.state = "finalized"
            return result
