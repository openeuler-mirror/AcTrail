# actrailweb 前端架构

本目录记录 `actrailweb` 前端的当前实现边界、控件与页面布局、运行时链路以及源码组织。设计尚未落地时，必须在对应文档中明确标为“目标设计”，不能覆盖当前实现描述。

## 文档索引

| 文档 | 内容 |
|---|---|
| [控件与页面布局](controls-and-layout.md) | 应用壳、Workspace、共享控件、关键页面布局与响应式规则 |
| [Navigator 与 Workspace](controls/navigation-workspace.md) | 导航控件边界、状态归属，以及 Trace 当前的两级导航结构 |
| [运行时与交付](runtime-and-delivery.md) | 技术边界、状态与请求流、Vite 构建和 Rust binary 嵌入 |
| [代码布局](code-layout.md) | 前端源码目录职责、依赖方向和文档中的路径约定 |

图源和渲染图统一位于 [`assets/`](assets/)。PUML 是图的源文件，PNG 是 Markdown 使用的渲染结果。

## 系统边界

`actrailweb` 前端是面向安全分析与本地运行时管理的 Vue 单页应用。构建产物嵌入 `actrailweb` Rust binary，并通过同一个 HTTP 服务提供页面资源和 `/api/*` 数据。服务端边界见 [Web Runtime](../web-runtime.md)。
