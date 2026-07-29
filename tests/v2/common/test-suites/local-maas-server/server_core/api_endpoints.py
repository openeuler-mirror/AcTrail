from __future__ import annotations

from dataclasses import dataclass

from protocol import ProtocolAdapter, ProtocolRegistry


@dataclass(frozen=True, slots=True)
class RestApi:
    service: str
    method: str
    path: str


@dataclass(frozen=True, slots=True)
class ProtocolEndpoint:
    protocol: str
    url: str


class ApiEndpoints:
    _HEALTH_PATH = "/healthz"

    def __init__(self, registry: ProtocolRegistry):
        self._registry = registry
        post_routes: dict[str, ProtocolAdapter] = {}
        model_routes: set[str] = set()
        for adapter in registry.adapters:
            if adapter.canonical_path not in adapter.paths:
                raise ValueError(
                    f"{adapter.name} canonical endpoint is not registered"
                )
            for path in adapter.paths:
                normalized = path.rstrip("/") or "/"
                if normalized in post_routes:
                    raise ValueError(f"duplicate protocol endpoint: {normalized}")
                post_routes[normalized] = adapter
            model_routes.update(
                path.rstrip("/") or "/" for path in adapter.model_paths
            )
        self._post_routes = post_routes
        self._model_routes = frozenset(model_routes)

    def resolve(
        self, method: str, request_path: str
    ) -> ProtocolAdapter | None:
        if method != "POST":
            return None
        return self._post_routes.get(request_path.rstrip("/") or "/")

    def supports_models(self, request_path: str) -> bool:
        return (request_path.rstrip("/") or "/") in self._model_routes

    def supports_health(self, request_path: str) -> bool:
        return (request_path.rstrip("/") or "/") == self._HEALTH_PATH

    def describe(self, origin: str) -> dict[str, str]:
        return {
            adapter.name: f"{origin}{adapter.canonical_path}"
            for adapter in self._registry.adapters
        }

    def describe_protocol_endpoints(
        self,
        origin: str,
    ) -> tuple[ProtocolEndpoint, ...]:
        return tuple(
            ProtocolEndpoint(
                protocol=adapter.name,
                url=f"{origin}{adapter.canonical_path}",
            )
            for adapter in self._registry.adapters
        )

    def describe_rest_apis(self) -> tuple[RestApi, ...]:
        routes = [RestApi("Local MaaS", "GET", self._HEALTH_PATH)]
        for adapter in self._registry.adapters:
            protocol_routes = [
                RestApi(adapter.service_name, "POST", path)
                for path in sorted(adapter.paths)
            ]
            protocol_routes.extend(
                RestApi(adapter.service_name, "GET", path)
                for path in sorted(adapter.model_paths)
            )
            routes.extend(
                sorted(
                    protocol_routes,
                    key=lambda route: (route.method, route.path),
                )
            )
        return tuple(routes)
