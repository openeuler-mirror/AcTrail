# 回归测试

需要 Python 3.11 或更高版本。

## 运行入口

| 操作 | 命令 |
| --- | --- |
| 列出测例 | `python3 tests/e2e/regression/test_all.py --list` |
| 运行默认集合 | `sudo -E python3 tests/e2e/regression/test_all.py` |
| 运行指定测例 | `sudo -E python3 tests/e2e/regression/test_all.py --case probe_claude_llm` |
| 查看参数 | `python3 tests/e2e/regression/test_all.py --help` |
| 独立运行 | `sudo -E python3 tests/e2e/regression/<测例目录>/run_e2e.py` |

## 常用参数

| 参数 | 用途 |
| --- | --- |
| `--case NAME` | 选择测例，可重复指定 |
| `--bin-dir PATH` | 指定 AcTrail 二进制目录 |
| `--profile PATH` | 显式加载指定测试配置；省略时不加载 profile |
| `--fail-fast` | 首个失败后停止 |
| `--no-cleanup` | 保留运行数据和日志 |
| `--log-dir PATH` | 指定日志目录 |
| `--work-root PATH` | 指定运行目录 |

## 结果

| 结果 | 含义 |
| --- | --- |
| 通过 | 测例检查通过 |
| 失败 | 执行或检查失败，详细原因见日志 |
| 跳过 | 未满足运行条件，输出中显示原因 |

日志位于 `<log-dir>/<case>.log`，使用 `--no-cleanup` 保留。

## 测例操作文档

| 测例 | 中文操作文档 |
| --- | --- |
| `checkpoint_btrfs` | [`checkpoint_btrfs/README.zh.md`](checkpoint_btrfs/README.zh.md) |
| `probe_claude_llm` | [`probe_claude_llm/README.zh.md`](probe_claude_llm/README.zh.md) |
| `probe_claude_mcp` | [`probe_claude_mcp/README.zh.md`](probe_claude_mcp/README.zh.md) |
| `probe_codex_llm` | [`probe_codex_llm/README.zh.md`](probe_codex_llm/README.zh.md) |
| `probe_codex_mcp` | [`probe_codex_mcp/README.zh.md`](probe_codex_mcp/README.zh.md) |
| `probe_dcode_llm` | [`probe_dcode_llm/README.zh.md`](probe_dcode_llm/README.zh.md) |
| `probe_pi_llm` | [`probe_pi_llm/README.zh.md`](probe_pi_llm/README.zh.md) |
| `probe_qodercli_llm` | [`probe_qodercli_llm/README.zh.md`](probe_qodercli_llm/README.zh.md) |
| `probe_xiaoo_llm` | [`probe_xiaoo_llm/README.zh.md`](probe_xiaoo_llm/README.zh.md) |
| `command_policy_xiaoo` | [`command_policy_xiaoo/README.zh.md`](command_policy_xiaoo/README.zh.md) |
| `virtual_container` | [`virtual_container/README.zh.md`](virtual_container/README.zh.md) |
| `virtual_container_xiaoo_concurrency` | [`virtual_container_xiaoo_concurrency/README.zh.md`](virtual_container_xiaoo_concurrency/README.zh.md) |
| `sandbox_resource_alert_host` | [`sandbox_resource_alert_host/README.zh.md`](sandbox_resource_alert_host/README.zh.md) |
| `sandbox_oom_killed_alert_host` | [`sandbox_oom_killed_alert_host/README.zh.md`](sandbox_oom_killed_alert_host/README.zh.md) |
| `resource_metrics_cgroup` | [`resource_metrics_cgroup/README.zh.md`](resource_metrics_cgroup/README.zh.md) |
| `resource_storage_retention` | [`resource_storage_retention/README.zh.md`](resource_storage_retention/README.zh.md) |
| `execution_isolation_firecracker` | [`execution_isolation_firecracker/v2/README.zh.md`](execution_isolation_firecracker/v2/README.zh.md) |
| `execution_isolation_stratovirt` | [`execution_isolation_stratovirt/v2/README.zh.md`](execution_isolation_stratovirt/v2/README.zh.md) |
| `execution_isolation_cloud_hypervisor` | [`execution_isolation_cloud_hypervisor/v2/README.zh.md`](execution_isolation_cloud_hypervisor/v2/README.zh.md) |
| `container_auto` | [`container_auto/README.zh.md`](container_auto/README.zh.md) |
| `container_agent_xiaoo` | [`container_agent_xiaoo/README.zh.md`](container_agent_xiaoo/README.zh.md) |
| `semantic_action_boundaries` | [`semantic_action_boundaries/README.zh.md`](semantic_action_boundaries/README.zh.md) |
| `otel_jsonl_action_filter` | [`otel_jsonl_action_filter/README.zh.md`](otel_jsonl_action_filter/README.zh.md) |
| `otel_http` | [`otel_http/README.zh.md`](otel_http/README.zh.md) |
| `project_subagent_trajectory` | [`project_subagent_trajectory/README.zh.md`](project_subagent_trajectory/README.zh.md) |
| `plugin_activity_anomaly` | [`activity_anomaly/README.zh.md`](activity_anomaly/README.zh.md) |
| `alert_forwarding` | [`alert_forwarding/README.zh.md`](alert_forwarding/README.zh.md) |
| `tool_consecutive_failure_alert` | [`tool_consecutive_failure_alert/README.zh.md`](tool_consecutive_failure_alert/README.zh.md) |
| `context_compact` | [`context_compact/README.zh.md`](context_compact/README.zh.md) |

## 添加测例

在测例目录的 `case.py` 定义 `TestCase` 的具体子类，声明唯一的 `case_id` 和 `description`，并实现构造函数与 `run()`。元类在导入时自动登记，Factory 按 `case_id` 找到测例类并调用构造函数。`case.py` 可以 re-export 子目录里的具体类。

所有测例通过同一个构造接口接收运行目录（`Case(directories: TestRunDirectories)`）。目录信息包含仓库目录（`repo`）、AcTrail 二进制目录（`bin_dir`）和本测例工作目录（`work_dir`）。测例在内部结合目录、环境变量和默认值构造自己的配置，配置解析由所属测例的 Config 类负责。Factory 传递目录信息，测例配置字段的变化由测例内部处理。

总入口自动导入各测例目录的 `case.py`。新测例默认加入默认集合，需要显式选择运行的测例设置 `default_enabled = False`。`--list` 列出全部登记测例，`--case` 按 `case_id` 选择。

测例标识是抽象接口（`TestCase.case_id`），具体测例通过类属性实现。元类只登记已实现全部抽象接口的类。抽象基类可保留未实现的接口，由具体子类补齐。重复标识会使启动失败。采集、查询等工具类以及由测例调用的场景对象按各自职责实现，无须继承 `TestCase`。

执行按 `case_id` 排序，`skip_if_skipped` 声明的前置测例排在使用它的测例之前。显式选择测例时只运行所选集合，跳过传播依据本次已完成的结果。

### 选择与启动 Agent

需要自动选择 Agent 的测例调用 `AgentFactory.auto(repo, test_context)`。Factory 按候选顺序查找程序，通过真实模型请求检查可用性，返回第一个可用的 Agent 实例。用 `kinds` 指定候选类型及顺序。所有候选均不可用时返回 `None`，测例据此报告跳过（`SKIPPED`）。测试目标指定了 Agent 类型时，直接使用 `AgentFactory.xiaoo(...)` 等构造入口。

同一次回归运行复用可用性检查的成功与失败结果（`AgentAvailability`）。结果按 Agent 类型和可执行路径索引，测试上下文结束后释放。该次运行期间保持模型与认证配置稳定。每轮任务创建新的 Agent 实例，共享同一测试上下文中的可用性结果。

### 设置工具权限与轮次

根据场景需要，在一次受观测启动的配置（`LaunchOptions`）中设置工具权限和轮次上限，并通过 `runtime.launch_scope(options)` 应用。

| 场景要求 | 启动配置 |
| --- | --- |
| 只读取和检索文件 | `tool_access=ToolAccess.READ_ONLY` |
| 允许修改文件 | `tool_access=ToolAccess.WRITABLE` |
| 完成纯文本任务 | `tool_access=ToolAccess.NONE` |
| 设置模型轮次上限 | `max_turns` 为正整数 |
| 沿用 Agent 构造参数 | 对应字段为 `None`，也是未设置时的值 |

显式设置的启动选项覆盖适配器中对应的工具或轮次参数，作用域退出后恢复外层配置。各 Agent 按 CLI 实际支持的能力应用选项。需要强制限制工具或轮次的测例，应选择能够应用相应限制的 Agent。

#### Agent 能力对应

| Agent | 只读工具 | 可写工具 | 无工具 | 最大轮次 |
| --- | --- | --- | --- | --- |
| XiaoO | `file_read`、`glob`、`grep` | CLI 默认工具 | `--no-tools` | `--max-turns` |
| Claude | `Read`、`Glob`、`Grep` | 默认工具 | 空工具集合 | `--max-turns` |
| Pi | `read`、`grep`、`find`、`ls`，关闭 MCP | 内置工具集合 | `--no-tools`，关闭 MCP | CLI 默认 |
| Codex | `read-only` sandbox | `workspace-write` sandbox | CLI 默认 | CLI 默认 |
| Qoder CLI | `Read`、`Glob`、`Grep` | 默认工具 | 空工具集合 | CLI 默认 |
| Dcode | CLI 默认 | CLI 默认 | CLI 默认 | 单次非交互模式支持 `--max-turns` |
| OpenCode | CLI 默认 | CLI 默认 | CLI 默认 | CLI 默认 |
| Grok ACP | CLI 默认 | CLI 默认 | CLI 默认 | CLI 默认 |

表中的 CLI 默认表示适配器沿用该 CLI 的现有行为。
