# Claude Subagent LLM Trajectory 真实回归

本测例使用刷新后的默认 daemon 配置，加载 OTel HTTP 插件并运行真实 Claude
Code。主 Agent 必须通过内建 `Agent`（部分版本显示为 `Task`）工具启动
`general-purpose` subagent；只有 subagent 可以在独立 Git fixture 中执行
`git rev-parse HEAD`。

## Quick Run

```bash
sudo -E python3.11 tests/v2/regression/llm_trajectory_claude/run_e2e.py
```

或通过总入口运行：

```bash
sudo -E python3.11 tests/v2/regression/test_all.py --case llm_trajectory_claude
```

需要：

- release 产物可由公共 runner 安装；
- `claude` 在 `PATH`，或设置 `CLAUDE_E2E_BINARY`；
- Claude 已配置可用凭据和模型；可选设置 `CLAUDE_E2E_MODEL`；
- 主机满足真实 Agent TLS/plaintext capture 的 eBPF 条件。

## 验证顺序

测试先验证真实场景，不使用 trajectory ID 推断角色：

1. Claude 最终输出包含随机 answer marker 和 fixture commit；
2. 主 Agent 的 `llm.response.tool_calls_json` 包含 `Agent` 或 `Task`；
3. 带随机 delegated marker 的 subagent response 调用 `Bash`；
4. trace 中存在成功的 `git rev-parse HEAD` command action；
5. 原始 request body 证明主 Agent 和 subagent 各有严格 history 前缀后继。

场景成立后才验证产品结果：

- main 和 subagent 使用两个不同 trajectory ID；
- 两条 trajectory 的第二轮分别继承第一轮 ID，且 parent action 正确；
- action attribute、lineage API、trajectory API 与 OTel metadata-only 导出一致；
- trajectory API 按顺序返回各自的两个节点；
- OTel 仅导出 `llm.request`，通过有界 flush 和轮询等待目标 action。

测试不要求整条 trace 恰好只有四个 LLM request，以容纳 Claude 版本可能增加的
独立后台请求；但四个 main/subagent 目标 request 必须唯一且证据完整。

## 环境变量

- `CLAUDE_E2E_BINARY`：Claude 可执行文件；
- `CLAUDE_E2E_MODEL`：真实调用模型，未设置时读取 `ANTHROPIC_MODEL`，再回退
  `sonnet`；
- `CLAUDE_TRAJECTORY_E2E_LAUNCH_TIMEOUT_SECONDS`：Claude 整体超时，默认由公共
  runner 提供；
- `CLAUDE_TRAJECTORY_E2E_DRAIN_ATTEMPTS`：trace、场景和 OTel 轮询次数；
- `CLAUDE_TRAJECTORY_E2E_DRAIN_INTERVAL_SECONDS`：轮询间隔；
- `CLAUDE_TRAJECTORY_E2E_REQUEST_CONTENT_MAX_BYTES`：单 request 重建上限，默认
  16 MiB。
