# 回归测试：程序执行与模型请求的完成记录

## 测试目标

验证 AcTrail 能记录程序启动成功、启动失败、非零退出和模型请求完成的结果。每次执行和请求具有独立的动作标识（action ID），同一条记录在存储与在线导出中使用相同标识，正常导出时每条记录出现一次。

## 场景设计

通过 AcTrail 直接启动真实 Agent，完成一次文本任务，检查程序执行、Agent 身份、模型请求及退出记录。启动前提供已知 Agent 二进制路径，使 AcTrail 能完成程序分析和探针准备。工具权限设为无工具，轮次上限由测试配置指定（`LaunchOptions.tool_access`、`max_turns`），各 Agent 按自身支持的参数应用。

另运行三个有明确实际结果的命令场景。

- 尝试启动一个没有执行权限的文件，验证启动失败被记录。
- 仅启用内核观测，运行成功退出的程序（eBPF、`/bin/true`），验证仅有执行结果事件时也能形成执行记录。
- 运行以指定非零退出码结束的程序（`exit 17`），验证启动成功与退出失败分别记录。

以命令的实际返回结果核对观测状态。通过查询工具读取持久化记录（`actrailviewer`），通过在线导出文件读取完成记录（OTEL JSONL），按本次启动取得的 trace ID 和动作标识核对两处数据。

## 测试依赖

| 依赖 | 要求 | 用途 |
| --- | --- | --- |
| Python | Python ≥ 3.11 | 运行测试执行器 |
| 主机环境 | Linux、root 权限、可工作的 eBPF 与 seccomp | 采集程序执行和退出 |
| AcTrail | 可运行的 release 产物及内置 OTEL JSONL 导出器 | 启动采集、查询记录及在线导出 |
| Agent | 可运行的真实 Agent，以及对应的已知二进制路径 | 执行模型任务并在启动前准备探针 |
| 模型服务 | 已配置认证且服务可访问 | 完成真实模型请求 |
| 系统命令 | `/bin/bash`、`/bin/true` | 产生结果明确的执行与退出行为 |

## 测试步骤摘要

1. **检查 Agent 可用性。** 选择能够完成真实模型请求的 Agent。没有可用 Agent 时，测例标记为跳过（`SKIPPED`）。
2. **准备观测环境。** 刷新隔离的默认配置（`actrailctl init -f`），登记已知 Agent 程序路径，启动守护进程并启用本测例所需的在线导出。
3. **执行 Agent 任务。** 通过 AcTrail 启动 Agent（`actrailctl launch`），记录 trace ID，提交文本任务并确认任务完成。
4. **核对 Agent 记录。** 等待本次执行和导出结束，查询同一 trace 的程序执行、模型请求、身份与退出记录，并核对存储和导出的动作标识。
5. **执行启动失败场景。** 创建没有执行权限的文件并尝试启动，确认命令失败，检查该文件对应的执行记录。
6. **执行仅内核观测场景。** 禁用执行前通知（seccomp notify），启用内核观测（eBPF），运行 `/bin/true`，确认返回成功并检查执行记录。
7. **执行非零退出场景。** 运行 `exit 17`，确认实际退出码为 17，检查该进程的执行和退出记录。

| 检查项 | 条件 | 预期结果 |
| --- | --- | --- |
| 成功启动的程序 | Agent 已成功启动 | 执行记录（`process.exec`）包含实际程序。记录中的启动路径（`exec.path`）与执行结果中的程序（`process.executable`）对应同一次启动 |
| 执行证据 | 同一次启动同时采到启动请求与执行结果 | 两类证据（`process.exec.intent`、`process.exec.completed`）归入同一条执行记录（`process.exec`），使用一个动作标识 |
| 仅有执行结果 | 禁用 seccomp notify，`/bin/true` 实际返回 0 | 该程序有且仅有一条执行记录，其证据为执行结果（`process.exec.completed`） |
| 启动失败 | 无执行权限的目标文件未能启动 | 该文件有且仅有一条失败执行记录，目标路径等于实际文件路径（`exec.path`），结果为失败（`exec.result=failed`），错误返回值为负数（`result`） |
| 非零退出 | 程序已启动并实际返回 17 | 保留程序的执行记录，并导出一条属于同一进程的退出记录（`process.exit`），退出码为 17，导出状态为错误（`STATUS_CODE_ERROR`） |
| 模型请求 | 真实 Agent 完成模型任务 | 至少有一条模型请求记录（`llm.request`）。各次模型请求使用独立动作标识，普通服务探测请求不归类为模型请求 |
| Agent 身份与退出 | 本次 Agent 进程已结束 | 该进程有一条身份记录（`agent.identity`），在线导出一条进程退出记录和一条 Agent 退出记录（`process.exit`、`agent.exit`） |
| 单次导出 | Agent 场景采集与导出已完成 | 同一 Agent 进程的每条受检记录具有非空动作标识，每个标识在导出结果中出现一次 |
| 持久化与导出 | 核对执行、命令、身份与模型请求记录 | 两处的动作标识和类型逐条对应（`process.exec`、`command.invocation`、`agent.identity`、`llm.request`），没有遗漏或额外记录 |
| 退出记录去向 | 进程和 Agent 已退出 | 退出记录（`process.exit`、`agent.exit`）可在线读取，持久化动作存储中没有这两类记录 |

## 快速运行

### 运行指令

从仓库根目录执行。

```bash
sudo -E python3 tests/e2e/regression/test_all.py \
  --case semantic_action_boundaries
```

也可直接运行测例入口。

```bash
sudo -E python3 tests/e2e/regression/semantic_action_boundaries/run_e2e.py
```

### 测试配置

| 环境变量 | 未设置时的行为 | 用途 |
| --- | --- | --- |
| `SEMANTIC_ACTION_BOUNDARIES_E2E_AGENT_MAX_TURNS` | 传入 `None`，沿用 Agent 默认轮次设置 | 设置时限制 Agent 任务轮次，各 Agent 按自身能力应用 |
| `SEMANTIC_ACTION_BOUNDARIES_E2E_COMMAND_TIMEOUT_SECONDS` | 使用测试框架默认值 | 限制 AcTrail 命令等待时间 |
| `SEMANTIC_ACTION_BOUNDARIES_E2E_LAUNCH_TIMEOUT_SECONDS` | 使用测试框架默认值 | 限制 Agent 和受控命令等待时间 |
| `SEMANTIC_ACTION_BOUNDARIES_E2E_WEB_PORT` | 自动选择可用端口 | 指定测试 Web 服务端口 |

## 手动检查观测记录

运行时保留本轮数据与日志。

```bash
sudo -E python3 tests/e2e/regression/test_all.py \
  --case semantic_action_boundaries \
  --no-cleanup
```

从本轮输出取得配置路径和 trace ID，使用查询工具（`actrailviewer`）查看该 trace 的动作。打开工作目录中的在线导出文件（`semantic-actions.jsonl`），按同一 trace 和进程筛选记录，再逐项核对上表中的动作标识、类型、结果与退出码。
