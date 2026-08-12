from __future__ import annotations

from abc import ABC, abstractmethod

from ..model import ResponseTemplate, ScenarioRequest
from ..scenario_generator.interface import GenerationOptions


class ToolAliasConversionError(RuntimeError):
    pass


class ToolAliasConverter(ABC):
    def canonicalize_template(
        self,
        template: ResponseTemplate,
    ) -> ResponseTemplate:
        return template

    @abstractmethod
    def generation_options(
        self,
        request: ScenarioRequest,
    ) -> GenerationOptions:
        raise NotImplementedError

    @abstractmethod
    def convert(
        self,
        template: ResponseTemplate,
        request: ScenarioRequest,
    ) -> ResponseTemplate:
        raise NotImplementedError
