# OpenCode Subagent LLM Trajectory 真实回归

本测例启动刷新后的默认 `actraild` 配置，加载内置 OTel HTTP 插件，并运行真实
OpenCode。主 Agent 被要求通过 `task` 工具启动 `general` subagent，由 subagent 在
测例创建的独立 Git fixture 中执行 `git rev-parse HEAD`。

## Quick Run

```bash
sudo -E python3.11 tests/v2/regression/llm_trajectory/run_e2e.py
```

也可以通过总入口运行：

```bash
sudo -E python3.11 tests/v2/regression/test_all.py --case llm_trajectory
```

需要：

- release 产物可由公共 runner 安装；
- `opencode` 在 `PATH`，或设置 `OPENCODE_E2E_BINARY`；
- OpenCode 已配置可用模型和凭据；可选 `OPENCODE_E2E_MODEL=provider/model`；
- 主机满足真实 Agent TLS/plaintext capture 的 eBPF 条件。

## 验证顺序

测试先验证场景本身，不使用 trajectory ID 反推角色：

1. OpenCode 主任务成功返回 fixture commit；
2. 找到 title-generation LLM call；
3. 主 Agent 的 `llm.response.tool_calls_json` 包含 `task`；
4. 实际出现带唯一 delegated token 的 subagent request；
5. subagent response 调用 shell 工具，且 trace 中存在成功的
   `git rev-parse HEAD` command action；
6. 从重建 request body 验证主 Agent 和 subagent 各自的后继是严格 history 前缀追加。

只有这些前置条件成立后，才验证产品结果：

- title、main、subagent 使用三个不同 trajectory ID；
- main 与 subagent 的第二轮分别继承第一轮 ID，且 lineage parent 正确；
- title trajectory 无 parent 且只有一个节点；
- 五个目标 request 的 action attributes、lineage API 与 OTel metadata-only 导出中的
  trajectory ID 一致；
- OTel 插件只启用 `llm.request`，通过配置替换触发有界 flush，并轮询等待全部目标
  action，而不是依赖固定长 sleep。

失败输出明确区分：

- `infrastructure-failure`：daemon、Web、插件、OpenCode 或 trace 生命周期失败；
- `scenario-precondition-failure`：模型没有实际形成预期 title/subagent/tool 拓扑；
- `product-assertion-failure`：场景成立，但 trajectory、parent、查询或 OTel 导出错误。

OpenCode 的真实模型行为不是确定性的。Title call 缺失、模型拒绝使用 task 或主 Agent
自行执行 Git 都属于场景失败，不会降级为跳过，也不会继续做无依据的 trajectory 断言。

## 集中超时配置

- `LLM_TRAJECTORY_E2E_LAUNCH_TIMEOUT_SECONDS`：OpenCode 整体运行超时，默认 180 秒；
- `LLM_TRAJECTORY_E2E_DRAIN_ATTEMPTS`：trace、场景和 OTel 轮询次数，默认 30；
- `LLM_TRAJECTORY_E2E_DRAIN_INTERVAL_SECONDS`：轮询间隔，默认 1 秒；
- `LLM_TRAJECTORY_E2E_REQUEST_CONTENT_MAX_BYTES`：单 request 重建上限，默认 16 MiB。
