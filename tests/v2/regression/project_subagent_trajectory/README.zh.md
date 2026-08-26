# LLM 轨迹图与项目子代理回归

## 测试目标

本用例使用真实 OpenCode、Claude 或 xiaoO 执行一个多子代理任务，验证 LLM 请求从
采集、语义投影、lineage 持久化到 Web 轨迹图 API 和 OTel 导出的完整链路。

本次新增的核心回归对象是：

```text
GET /api/traces/{trace_id}/llm-trajectories
```

测试不把接口返回值与手写固定快照比较，而是以同一 trace 中持久化的
`llm.request` action 和逐请求 lineage endpoint 为独立事实源，重新计算预期节点、
边和统计值，再与整图接口逐项比较。这样可以同时发现漏节点、错误连边、聚合错误和
字段语义退化。

## 场景与职责边界

真实 Agent 在隔离的 `sorting-workspace/` 中完成一个可在 Web 页面直接分析的排序任务：

回测发送的核心中文任务与手工演示一致：

```text
现在派生2个subagent，一个编写一下冒泡排序，一个编写一个二分排序。然后主agent测试哪个速度快。
```

自动回测会追加英文约束，将“二分排序”固定为二分插入排序，并固定输出文件、并发启动和
benchmark 输入要求，以降低真实模型输出波动。

1. 同时派生两个子代理；
2. 子代理分别实现并自测冒泡排序和二分插入排序；
3. 主 Agent 读取两份实现，生成 benchmark，并使用至少三组相同的确定性输入测试速度；
4. 主 Agent 汇总哪个实现更快。

测试只把该任务作为产生真实 LLM 请求、工具结果和并发上下文的 workload，不断言模型回答
中的具体耗时或算法复杂度。排序文件写入回测工作目录，不修改 AcTrail 源码仓。

真实图必须支持以下 Web 分析：

- 至少三条独立 LLM 上下文（主上下文和两个子任务；Agent 的标题辅助上下文允许额外出现）；
- 至少一条上下文存在连续 append；
- 同一连续上下文中的 `tool_result_count` 不倒退，且至少增长一次；
- 按纳秒时间排序后，至少一条 trajectory 被另一条 trajectory 穿插；
- 成功场景中全部节点为 `success + complete`，因此页面不应出现 incomplete/error 节点；
- append/fork/duplicate 的实际数量仍由持久化 lineage 重算，不预设模型一定产生 fork 或 duplicate。

严格 fork、duplicate、截断响应和 HTTP 错误不能通过自然语言稳定驱动真实 Agent，因此仍由
确定性图断言/协议专项用例覆盖；本用例不会为了凑数伪造真实 Agent 行为。当前版本也不启用
related 弱关系和 compaction 自动识别。

## 自动化覆盖

### LLM 调用与投影完整性

- 全部 `llm.call`、`llm.request`、`llm.response` 必须一一配对；
- LLM action 必须进入终态；
- 每个 request 的 action attribute、lineage 和单轨迹 endpoint 必须一致；
- 每条 trajectory 的 position 连续，parent 形成连续链；
- 排序子代理场景必须产生至少三条独立 trajectory。

### 整图 API 契约

- `trace_id` 必须与请求一致，`partial` 必须为 `false`；
- 图节点集合必须与全部持久化 `llm.request` 精确一致，不得遗漏、重复或混入其他节点；
- 每个节点的 trajectory、position、transition、start reason、inference version、
  process、status 和 completeness 必须与 action/lineage 一致；
- `model`、`classifier_id`、`block_count`、`user_message_count` 和
  `tool_result_count` 必须保持属性语义，其中缺失值和数值 `0` 不得混淆；
- `start_time_unix_nanos` 必须为十进制字符串，节点必须按纳秒时间和 action ID
  稳定排序；
- append/fork 边必须完全由 `parent_action_id`/`forked_from_action_id` 推导，边集合、
  类型、置信度和顺序必须一致；
- node、trajectory、append、fork、duplicate 计数及两个比例必须可由事实源重算；
- capabilities 必须明确声明 strict-prefix 已启用，而 related 和 compaction detection
  在当前版本未启用。

### OTel 导出

- 每个 request/response 必须且只能导出一个 OTel span；
- request span 的 trajectory ID 必须与持久化 lineage 一致；
- HTTP 失败 exchange 与成功 exchange 使用同样的完整性要求。

## 自动运行

### 前提条件

以下命令会安装最新 release、清理测试运行时数据并启动 daemon/Web/OTel receiver。
只能在测试机执行，不要与生产实例共用运行目录。

需要：

- Rust/Cargo release 构建环境；
- root 权限；
- `curl`；
- 已登录 Agent 为可选前提；自动发现不到任何可用 Agent 时真实场景规范跳过；
- 对应 provider 的网络和凭据已经配置，但凭据不得写入仓库或 README。

### Quick Run

从仓库根目录运行自动 Agent 选择场景：

```bash
sudo -E python3.11 \
  tests/v2/regression/project_subagent_trajectory/run_e2e.py
```

通过统一 runner 运行：

```bash
sudo -E python3.11 tests/v2/regression/test_all.py \
  --case project_subagent_trajectory
```

选择 Claude 或 xiaoO：

```bash
sudo -E PROJECT_SUBAGENT_TRAJECTORY_E2E_AGENT_BINARY=claude \
  python3.11 tests/v2/regression/project_subagent_trajectory/run_e2e.py

sudo -E PROJECT_SUBAGENT_TRAJECTORY_E2E_AGENT_BINARY=xiaoo \
  python3.11 tests/v2/regression/project_subagent_trajectory/run_e2e.py
```

`agent_binary` 是 Agent 类型选择器，不是文件路径。实际路径分别由
`OPENCODE_E2E_BINARY`、`CLAUDE_E2E_BINARY`、`XIAOO_E2E_BINARY` 覆盖。
OpenCode 模型可由 `OPENCODE_E2E_MODEL` 配置；Claude 使用其通用模型配置；xiaoO
使用 `~/.config/xiaoo/config.toml`。

未设置 `PROJECT_SUBAGENT_TRAJECTORY_E2E_AGENT_BINARY` 时，回测按
`opencode → claude → xiaoo` 自动选择：二进制不存在或真实可用性检查失败都会继续尝试
下一个；三个候选全部不可用时返回 `SKIPPED`，完整回测不会因此失败。显式设置该变量时
只检查指定 Agent，便于定向复现。

### 关键环境变量

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `PROJECT_SUBAGENT_TRAJECTORY_E2E_AGENT_BINARY` | 未设置（自动） | 指定时只测 `opencode`、`claude` 或 `xiaoo` |
| `PROJECT_SUBAGENT_TRAJECTORY_E2E_WEB_HOST` | `127.0.0.1` | 测试 Web 监听地址 |
| `PROJECT_SUBAGENT_TRAJECTORY_E2E_WEB_PORT` | 自动分配 | 固定 Web 端口，手工检查时建议设置 |
| `PROJECT_SUBAGENT_TRAJECTORY_E2E_LAUNCH_TIMEOUT_SECONDS` | `180` | Agent 场景超时 |
| `PROJECT_SUBAGENT_TRAJECTORY_E2E_COMMAND_TIMEOUT_SECONDS` | `30` | 普通命令超时 |
| `PROJECT_SUBAGENT_TRAJECTORY_E2E_DRAIN_ATTEMPTS` | `30` | 等待异步持久化/导出的重试次数 |
| `PROJECT_SUBAGENT_TRAJECTORY_E2E_DRAIN_INTERVAL_SECONDS` | `1` | 重试间隔秒数 |
| `PROJECT_SUBAGENT_TRAJECTORY_E2E_TRACE_RANDOM_BYTES` | `3` | trace 名随机后缀字节数，范围 3 至 8 |
| `PROJECT_SUBAGENT_TRAJECTORY_E2E_XIAOO_MAX_TURNS` | `20` | xiaoO 主循环最大轮数 |
| `PROJECT_SUBAGENT_TRAJECTORY_E2E_REQUEST_CONTENT_MAX_BYTES` | `16777216` | request 内容读取上限 |

测试不会向 LLM prompt 注入随机 marker。随机后缀只用于筛选本轮 OTel span。

### 自动化执行步骤

1. 未指定 Agent 时按 `opencode`、`claude`、`xiaoo` 顺序检查，缺失或不可用就尝试下一个，全部不可用则 SKIPPED；
2. 启动全新的 daemon、Web API 和 OTel HTTP receiver；
3. 配置 request/response 完整导出；
4. 在隔离工作目录运行双排序子代理与主 Agent benchmark，并等待 trace 终态；
5. 验证 call 配对、lineage 和单轨迹 endpoint；
6. 请求整图 API，校验节点、边、统计和 capabilities，并验证独立上下文、append、工具结果增长、时间交错及完整状态；
7. flush OTel exporter，验证每个 request/response 恰好导出一次。

通过时结果中应包含：

```text
semantic-projection: PASSED
trajectory-graph-api: PASSED
web-analysis-scenario: PASSED
otel-export: PASSED
```

## 断言代码自测

该命令不启动 daemon、不调用真实 Agent，也不需要 root。它验证回测判据本身能够接受
正确图和真实排序分析形状，并拒绝缺失 fork 边、`null`/`0` 语义退化及 incomplete 节点：

```bash
python3 -m unittest \
  tests.v2.regression.project_subagent_trajectory.test_graph_assertion
```

## 手工验证

手工检查应与自动回测使用同一接口。固定 Web 端口并保留现场：

```bash
sudo -E PROJECT_SUBAGENT_TRAJECTORY_E2E_WEB_PORT=18089 \
  python3.11 tests/v2/regression/project_subagent_trajectory/run_e2e.py \
  --no-cleanup
```

从测试输出或日志中取得 `trace-<id>` 的数字部分：

```bash
TRACE_ID=<替换为数字 trace id>
curl -fsS \
  "http://127.0.0.1:18089/api/traces/$TRACE_ID/llm-trajectories" \
  > /tmp/actrail-llm-trajectory-graph.json
jq . /tmp/actrail-llm-trajectory-graph.json
```

基础契约快速检查：

```bash
jq -e --argjson trace_id "$TRACE_ID" '
  .trace_id == $trace_id
  and .partial == false
  and (.nodes | length) == .stats.node_count
  and ([.nodes[].id] | length) == ([.nodes[].id] | unique | length)
  and .capabilities.strict_prefix_edges == true
  and .capabilities.related_edges == false
  and .capabilities.compaction_detection == false
' /tmp/actrail-llm-trajectory-graph.json

jq '.stats, .edges, [.nodes[] | {
  id,
  trajectory_id,
  trajectory_position,
  transition,
  tool_result_count,
  process
}]' /tmp/actrail-llm-trajectory-graph.json
```

也可以在浏览器打开 `http://127.0.0.1:18089`，进入对应 trace 后选择
`LLM Trajectory` 标签页。页面节点数、轨迹数、append/fork 数应与上述 JSON 的
`stats` 一致；点击节点应打开现有 Action 详情面板。

`--no-cleanup` 会保留测试现场。检查结束后应按测试机既有运维流程清理，避免误删其他
并行测试或开发数据。

## 失败判定与排障

| 失败阶段 | 常见原因 | 优先检查 |
| --- | --- | --- |
| `agent_availability` | 候选 Agent 均未安装、未登录或 provider 不可达 | 此阶段应显示 SKIPPED；若指定了 Agent，只检查该 Agent |
| `agent-run` | 多子代理能力不可用或场景超时 | Agent 输出、launch timeout、网络 |
| `semantic-projection` | call 配对或 lineage 未完整持久化 | daemon 日志、action/lineage endpoint |
| `trajectory-graph-api` | 图漏节点、错误边、字段或统计退化 | 回测日志中的 observed/expected 差异 |
| `otel-export` | exporter 未 flush、重复或漏导出 | OTel receiver 与插件运行状态 |

统一 runner 的详细日志默认位于：

```text
/tmp/actrail-regression/logs/project_subagent_trajectory.log
```

失败时可使用 `--no-cleanup` 保留 workspace、runner log 和 runtime 数据库现场。不要通过
放宽节点、边或计数断言来适配失败结果；若产品契约有意变化，应同步更新设计文档、接口
实现、回测判据和本 README。

## 文件说明

- `run_e2e.py`：注册独立回测入口；
- `case.py`：编排环境、真实 Agent、图断言和 OTel 断言；
- `scenario.py`：启动真实多子代理 workload；
- `assertion.py`：以 action/lineage 为事实源校验整图 API；
- `test_graph_assertion.py`：断言逻辑的无外部依赖自测；
- `agent.py`：OpenCode、Claude、xiaoO 启动适配；
- `config.py`：环境变量和测试运行配置。
