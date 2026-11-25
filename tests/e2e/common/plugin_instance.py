"""Own one loaded plugin instance inside the current AcTrail runtime."""
from __future__ import annotations

from dataclasses import dataclass, replace
from pathlib import Path

from tests.e2e.common.actrail_runtime import ActrailRuntime


@dataclass(frozen=True)
class PluginInstanceSpec:
    manifest: Path
    instance_id: str
    grants: tuple[str, ...]
    plugin_config: Path | None = None


class PluginInstance:
    def __init__(self, spec: PluginInstanceSpec):
        self.spec = spec
        self.loaded = False

    def load_command(self, plugin_config: Path | None = None) -> list[Path | str]:
        spec = self.spec if plugin_config is None else replace(self.spec, plugin_config=plugin_config)
        arguments: list[Path | str] = [
            "plugin", "load", "--manifest", spec.manifest, "--instance", spec.instance_id,
        ]
        if spec.plugin_config is not None:
            arguments.extend(("--plugin-config", spec.plugin_config))
        for grant in spec.grants:
            arguments.extend(("--grant", grant))
        return ActrailRuntime.current().daemon_command(*arguments)

    def unload_command(self) -> list[Path | str]:
        return ActrailRuntime.current().daemon_command("plugin", "unload", "--instance", self.spec.instance_id)

    def status_command(self) -> list[Path | str]:
        return ActrailRuntime.current().daemon_command("plugin", "status", "--instance", self.spec.instance_id)

    def load(self, plugin_config: Path | None = None) -> None:
        if self.loaded:
            return
        result = ActrailRuntime.current().run_checked(self.load_command(plugin_config))
        # 加载命令成功后即接管资源，后续报告校验失败也必须卸载。
        self.loaded = True
        if f"loaded instance={self.spec.instance_id}" not in result.output:
            raise AssertionError("Plugin load did not report its instance: " + result.output)

    def unload(self) -> None:
        if self.loaded:
            ActrailRuntime.current().run_checked(self.unload_command(), echo=False)
            self.loaded = False

    def status(self) -> dict[str, str]:
        result = ActrailRuntime.current().run_checked(self.status_command(), echo=False)
        return dict(token.split("=", 1) for token in result.stdout.split() if "=" in token)

    def __enter__(self):
        try:
            self.load()
        except BaseException as error:
            self.__exit__(type(error), error, error.__traceback__)
            raise
        return self

    def __exit__(self, exception_type, exception, traceback) -> None:
        try:
            self.unload()
        except Exception as error:
            if exception is None:
                raise
            exception.add_note(f"plugin unload failed: {error}")
