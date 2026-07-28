# OTEL JSONL 插件设计文档

本目录收录 AcTrail 内置 `otel-jsonl` observation consumer 的设计文档。该插件在
trace 运行期间消费 semantic action，将选中的 action 编码为 OTLP JSON，并以每个
action 一行的 JSONL 形式异步写入本地文件。

## 阅读顺序

1. [内置插件候选发现设计](builtin-candidate-discovery.zh.md)
   - 解释 builtin 执行代码与可发现描述包之间的关系。
   - 定义 release 安装、Web 候选发现和插件生命周期。
   - 记录已经采纳并实现的候选包布局。

2. [Semantic action kind 选择策略设计](action-kind-selection.zh.md)
   - 定义 `action_kinds` 布尔映射及 `default` 语义。
   - 规定 consumer queue 前过滤、配置校验和上游 TTY 保护约束。
   - 明确插件未加载时不要求配置，本次不修改 `actrailviewer`。

3. [目标文件路径](targeted-file-paths.zh.md)
   - 索引整个插件从描述包、发现、加载到 JSONL 写出的关键路径。
   - 标识 plugin system、recording、export 和 Web 等关键邻接模块。
   - 记录目标设计需要新增或修改的文件落点。

## 文档职责

| 文档 | 性质 | 更新时机 |
| --- | --- | --- |
| `builtin-candidate-discovery.zh.md` | 已采纳的候选发现与安装设计 | builtin 包布局、发现规则或加载生命周期变化时 |
| `action-kind-selection.zh.md` | 规范性目标设计 | action 选择协议、默认策略、过滤位置或 viewer 复用方式变化时 |
| `targeted-file-paths.zh.md` | 整个插件的关键文件路径索引 | 插件路径、邻接模块或目标改动落点变化时 |
| `README.zh.md` | 本目录索引 | 新增、删除或重命名本目录文档时 |

README 只维护目录导航和文档职责，不承载具体方案。目标设计与当前实现不一致时，
应显式判断实现需要迁移，还是设计需要修订。
