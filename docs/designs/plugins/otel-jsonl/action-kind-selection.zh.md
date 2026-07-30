# OTEL JSONL Semantic Action Kind 选择策略设计

## 状态

已实现。

## 1. 背景

`otel-jsonl` 是 AcTrail 内置的 observation consumer。当前 live exporter 收到
semantic action batch 后，会遍历 batch 中的 action，并在没有 action kind 配置
过滤的情况下逐条发布到异步 JSONL route。recording export 层会在上游排除
`file.tty_io`，避免超高频 TTY 刷新进入 observation consumer 和 exporter；
OTLP codec 还会跳过已经失效的 action。除此之外，`file.read`、
`file.write`、`http.message`、`sse.event` 等高频 semantic action 都会进入 live
exporter。

这会产生三个问题：

1. 默认输出包含大量与 Agent/LLM 核心观测目标无关的 span。
2. 无关 action 在被丢弃前已经占用异步队列和编码资源，放大 backpressure。
3. 缺少显式协议时，用户无法从配置判断 exporter 会接收哪些可导出 kind。

本文定义一个显式的 `{ action kind -> bool }` 选择策略，用于 live `otel-jsonl`
插件及其 Web/API 配置协议。`file.tty_io` 是该协议之外的上游保护约束，不是一个
可以打开的 exporter 选项。

## 2. 文档地位

本文是 OTEL semantic action kind 选择行为的目标规范。

本文使用以下规范词：

- **必须**：实现不得违反。
- **禁止**：实现不得采用。
- **应该**：除非存在明确且可记录的工程理由，否则应遵循。
- **可以**：允许采用，但不是强制要求。

AcTrail 尚未达到 `v1.0`。本设计不保留旧版 `otel-jsonl` 配置兼容分支，也不要求
旧配置继续通过 schema 或运行时校验。

## 3. 目标

1. 每个允许进入 OTEL exporter 的 `SemanticActionKind` 都可以通过布尔值显式启用
   或关闭。
2. 未显式列出的 kind 使用统一的 `default` 值。
3. 官方默认配置列出当前所有可导出 kind，使可配置范围可以直接发现和核对。
4. action 必须在进入 exporter 异步队列前完成过滤。
5. live exporter 和 Web/API 必须使用同一个选择模型与匹配语义。
6. 未知 kind、错误类型和错误配置必须在加载或命令启动阶段 fail-fast。
7. 新增 `SemanticActionKind` 时，默认不得意外扩大遥测输出范围。
8. 除本文明确规定的上游 `file.tty_io` 保护外，不允许保留无配置入口的隐藏 kind
   allowlist 或 denylist。
9. Web 前端必须为 action kind map 提供可勾选的筛选控件。

## 4. 非目标

本文不设计：

- action status、completeness、attribute 或路径级过滤。
- 同一 `action_id` 的 `in_progress`、terminal 或重复更新抑制。
- sampling、速率限制或基于 trace 的概率选择。
- OTLP/HTTP transport 或 collector 推送。
- raw eBPF event、payload bytes 或 diagnostic event 的导出策略。
- 对 `v1.0` 之前旧插件配置的自动迁移或兼容解析。
- `actrailviewer export-otel` 的离线 action kind 筛选。
- 旧 `[export.runtime]` 静态 OTEL JSONL 入口的兼容保留。

### 4.1 实现约束

实现必须遵循以下约束：

1. 不得删除、移动或绕过 recording 层现有 `file.tty_io` 上游过滤。
2. `file.tty_io` 不得出现在 `action_kinds`、JSON Schema 或 Web checkbox 中。
3. 普通 action kind 的选择必须位于 `consume()` 内，先完成 batch 一致性校验，再在
   `route.publish()` 和 exporter 异步队列之前过滤。
4. 插件未加载时不得读取或要求 `otel-jsonl` plugin config。
5. `otel-jsonl` 只通过插件生命周期启动，不保留 `[export.runtime]` 静态入口。
6. `actrailviewer export-otel` 不属于本文实现范围。

## 5. 配置协议

插件配置必须包含顶层 `[action_kinds]` table：

```toml
path = "/var/lib/actrail/export/live-spans.otlp.jsonl"
overwrite_enabled = true
queue_capacity = 1024
flush_every_spans = 1

[action_kinds]
default = false

"process.exec" = true
"process.exit" = true
"agent.identity" = true
"agent.exit" = true
"file.modify" = false
"file.read" = false
"file.write" = false
"file.bulk_read" = false
"fs.enumerate" = false
"http.message" = false
"llm.call" = true
"llm.request" = true
"llm.response" = true
"sse.stream" = false
"sse.event" = false
"enforcement.decision" = true
"process.fork_attempt" = false
"agent.invocation" = true
"command.invocation" = true
```

TOML 中的 action kind key 必须使用允许导出的 canonical
`SemanticActionKind::as_str()` 值。由于这些值包含 `.`，配置文件必须使用 quoted
key，避免 TOML 将其解释为嵌套路径。`file.tty_io` 不属于允许集合，配置该 key
必须失败。

### 5.1 数据模型

外部协议使用一个布尔映射，其中 `default` 是保留控制字段，其余 key 是 canonical
action kind：

```json
{
  "action_kinds": {
    "default": false,
    "process.exec": true,
    "process.exit": true,
    "agent.identity": true,
    "agent.exit": true,
    "llm.request": true,
    "llm.response": true
  }
}
```

内部模型应该在解析阶段把字符串 key 转换为 `SemanticActionKind`：

```rust
struct SemanticActionKindSelection {
    default_enabled: bool,
    overrides: BTreeMap<SemanticActionKind, bool>,
}
```

运行期匹配禁止重复执行字符串解析。为保持配置输出与测试快照的确定性，
`SemanticActionKind` 应提供稳定的 `Ord`/`PartialOrd` 顺序；不得使用进程间不稳定的
hash 迭代顺序生成协议文档。

### 5.2 `default` 语义

`default` 省略时必须按 `false` 处理。JSON Schema 中的 `default: false` 只是协议
描述，运行时解析器仍必须显式实现该默认值，不能依赖 validator 修改输入文档。

匹配函数必须等价于：

```rust
fn enabled(
    selection: &SemanticActionKindSelection,
    kind: SemanticActionKind,
) -> bool {
    selection
        .overrides
        .get(&kind)
        .copied()
        .unwrap_or(selection.default_enabled)
}
```

由此形成两种完整且无冲突的模式：

```toml
# 默认关闭，只打开少量核心动作
[action_kinds]
default = false
"llm.request" = true
"llm.response" = true
```

```toml
# 默认打开，只关闭已知高频动作
[action_kinds]
default = true
"file.read" = false
"http.message" = false
"sse.event" = false
```

当未来增加新的、允许进入 OTEL exporter 的 `SemanticActionKind` 时，未更新配置的
行为由 `default` 唯一决定。官方默认配置使用 `default = false`，因此新增可导出
kind 不会自动进入遥测输出。上游永久过滤的 kind 不参与此匹配。

### 5.3 必填与校验

顶层 `action_kinds` 必须是 schema 的 required property。旧配置缺少该 table 时必须
加载失败，不得回退到“全部导出”。

`action_kinds` 的 schema 必须：

1. 将 `default` 定义为 boolean，schema default 为 `false`。
2. 将每个当前允许导出的 canonical action kind 定义为可选 boolean property。
3. 设置 `additionalProperties: false`。
4. 拒绝字符串、整数等非 boolean 值。
5. 拒绝拼写错误、当前二进制未知或被上游永久过滤的 action kind。

官方配置应该显式列出当前所有可导出 kind，即使某个值与 `default` 相同。完整枚举
用于展示可配置范围；匹配语义仍允许用户配置只保留少量 override。

### 5.4 Web 前端约束

Web 前端必须将 `action_kinds` 中的 boolean action kind 显示为可勾选的筛选项，
并将勾选结果按同一个 `{ string -> bool }` 协议提交。禁止要求用户通过自由文本
编辑 action kind map，也禁止为 Web 单独定义另一套筛选协议。

## 6. 过滤位置

live exporter 必须在 `OtelJsonlObservationConsumer::consume()` 中、调用
`route.publish()` 之前应用选择策略：

```text
ObservationBatch
  → 校验 trace/action/link 一致性
  → action kind selection
      ├── false：计为策略过滤，不进入 route
      └── true：构造 export record
                  → async queue
                  → OTLP codec
                  → JSONL sink
```

禁止只在 OTLP codec 中过滤，因为此时 action 已经占用 exporter 队列。禁止写入
不含 span 的空 OTLP JSONL document 代表一次过滤。

选择策略只决定 action kind 是否进入 exporter。action 失效校验属于数据完整性
约束，不属于用户可配置 kind 过滤，可以继续独立拒绝无效 action。

## 7. 上游 TTY 保护

recording live export 路径对 `SemanticActionKind::FileTtyIo` 的现有过滤必须保留。
TTY 刷新频率可能远高于普通 semantic action；若把它交给 observation consumer，
即使随后在 exporter 内丢弃，也会占用上游分发、consumer 调度和内存资源。

因此：

- `file.tty_io` 禁止进入 `action_kinds` schema 和官方配置。
- 配置中出现 `file.tty_io` 必须在加载阶段失败。
- Web 禁止展示一个实际无法生效的 TTY checkbox。
- exporter selection 不承担 TTY 限流或兜底职责。
- 实现不得移动、删除或绕过 recording 层现有 TTY 过滤。

## 8. OTLP link 与父 span

过滤不得隐式启用 action 的父 action。一个启用的 child action 可以引用一个被
关闭的 parent span，这与 trace sampling 后出现缺失 parent 的情况一致。用户若
要求完整层级，应显式启用对应的父 kind，例如 `llm.call`、`command.invocation`
或 `process.exec`。

实现必须过滤掉 child 自身未启用的输出，但不得为了闭合 action tree 而绕过用户
配置扩大导出集合。

## 9. 共享 contract 与本次边界

选择策略不属于 JSONL 文件 sink 私有概念。共享 contract 应位于 export core，并
提供：

- canonical key 解析；
- schema/config DTO 到内部 selection 的转换；
- `enabled(SemanticActionKind)` 匹配；
- 已知 key 集合校验；
- 确定性的配置序列化顺序。

`otel-jsonl` 在 action 入队前调用该 contract。Web/API 必须使用相同的 JSON 对象
形状：

```json
{
  "action_kinds": {
    "default": false,
    "llm.request": true,
    "llm.response": true
  }
}
```

本次不修改 `actrailviewer export-otel`。后续如果离线导出增加 action kind 筛选，
必须复用该 contract，不得重新定义 include/exclude list 或另一套匹配优先级。

## 10. 插件启动边界

`otel-jsonl` 只通过插件生命周期启动，包括 `[plugins.startup]` 和运行期 plugin
load。旧 `[export.runtime]` 静态 OTEL JSONL route 不保留为兼容入口。

插件未加载时：

- 不读取 `otel-jsonl` plugin config；
- 不要求存在 `[action_kinds]`；
- 不创建 exporter route、异步队列或输出文件。

`action_kinds` 必填是“加载 `otel-jsonl` 插件”的前置条件，不是 daemon operator
配置的全局要求。

## 11. Schema 与版本

本次变更直接修改 `otel-jsonl.config.v1.schema.json` 和官方
`otel-jsonl.config.toml`。AcTrail 尚未达到 `v1.0`，不新增 v2 schema，不保留旧
schema 副本，也不接受缺少 `action_kinds` 的旧配置。

schema 中的 action kind property 集合必须与允许进入 OTEL exporter 的 canonical
字符串集合保持同步，并明确排除 `file.tty_io`。新增或重命名可导出 kind 时，必须
在同一个变更中更新：

1. 共享选择 contract 的解析；
2. JSON Schema properties；
3. 官方默认配置；
4. 本文中的完整配置示例；
5. `tests/v2/regression/plugins/otel-jsonl/` 中相应的代表性端到端场景。

`tests/v2/regression/plugins/otel-jsonl/` 必须验证代表性 action kind 组合、Web API
配置更新和实际 OTEL JSONL 输出。测试不要求枚举与 schema 的逐项相等检查、
boolean 或 kind 组合穷举，也不要求为现有通用 Web checkbox 建立独立测试框架。

## 12. 可观测性

`observed_records` 表示 observation consumer 收到的 action 数，不应被重新解释为
最终写出的 span 数。`dropped_records` 继续表示进入 route 后因队列或写入错误丢失
的 action 数。被 selection 正常过滤的 action 不是 delivery drop。

本次不为 action kind selection 扩展公共插件状态字段。实际导出集合由现有 v2
regression 读取 OTEL JSONL 验证。

## 13. 验收标准

1. 缺少 `[action_kinds]` 的插件配置无法通过 schema 或运行时加载校验。
2. `default` 缺失时按 `false` 处理。
3. 显式 `true` 的 kind 被导出，显式 `false` 的 kind 不进入异步 route。
4. 未列出的已知 kind 严格服从 `default`。
5. 未知 key、`file.tty_io` 和非 boolean 值在加载阶段失败。
6. 官方默认配置显式列出当前全部可导出 canonical action kind，且不列出
   `file.tty_io`。
7. recording 层现有 TTY 过滤保持不变，TTY 不进入 observation consumer。
8. 全部 kind 关闭时输出文件保持为空，不写入空 span document。
9. 过滤发生在 exporter queue 前，filtered action 不占用 route queue capacity。
10. `default = false` 时新增但未显式配置的可导出 kind 不会自动导出。
11. Web/API 能够读取、校验和提交 schema 声明的 action kind。
12. `tests/v2/regression/plugins/otel-jsonl/` 的现有代表性组合验证实际 OTEL JSONL
    只包含启用的 kind。
13. 插件未加载时不要求 `action_kinds`，也不启动 exporter。
