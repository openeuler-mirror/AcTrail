# 可配置 Agent 的项目子代理回归

该用例通过 `agent_binary` 选择真实 OpenCode、Claude 或 xiaoO，并要求主 agent 启动三个子代理。三个子代理分别统计 `crates/` 顶层目录、读取最新 commit 时间和读取当前分支，主 agent 只汇总结果。

三个子代理必须严格按单项任务分工，执行指定查询后立即返回，不允许扩展探索或检查其他子代理的结果；仅当指定命令失败时允许一次最小纠正。

项目事实仅用于产生真实、相互独立的子任务，不作为产品断言。测试不检查 agent 的答案、工具名称、工具参数、调用轮数或执行时间。

产品断言只依赖 AcTrail 持久化数据：全部 `llm.call`、`llm.request`、`llm.response` 必须一一配对；每个 request 的 action attribute、lineage 和 trajectory endpoint 必须一致；每条 trajectory 的 position 与 parent 必须连续；每个 request/response 必须且只能导出一个 OTel span。HTTP 失败 exchange 与成功 exchange 使用相同的完整性要求。

xiaoO 在 `spawn_subagent`、`join_subagent` 不可用或任一调用失败时，不允许主 agent 代做三个任务，只回复“没有”。该回复本身不作为产品断言。

测试不会向 LLM prompt 注入随机 marker。仅用于筛选本轮 OTel span 的 trace name 默认使用 3 字节随机后缀，可通过 `PROJECT_SUBAGENT_TRAJECTORY_E2E_TRACE_RANDOM_BYTES` 调整，允许范围为 3 至 8 字节。

`agent_binary` 是 Agent 类型选择器，不是可执行文件路径：

```bash
# 默认 OpenCode
PROJECT_SUBAGENT_TRAJECTORY_E2E_AGENT_BINARY=opencode \
python3 tests/v2/regression/project_subagent_trajectory/run_e2e.py

# Claude
PROJECT_SUBAGENT_TRAJECTORY_E2E_AGENT_BINARY=claude \
python3 tests/v2/regression/project_subagent_trajectory/run_e2e.py

# xiaoO
PROJECT_SUBAGENT_TRAJECTORY_E2E_AGENT_BINARY=xiaoo \
python3 tests/v2/regression/project_subagent_trajectory/run_e2e.py
```

实际二进制路径继续分别由 `OPENCODE_E2E_BINARY`、`CLAUDE_E2E_BINARY` 和 `XIAOO_E2E_BINARY` 覆盖。OpenCode 和 Claude 的模型分别使用 `OPENCODE_E2E_MODEL` 和 Claude 的通用模型配置；xiaoO 使用 `~/.config/xiaoo/config.toml`。xiaoO 主循环默认最多运行 20 轮，可通过 `PROJECT_SUBAGENT_TRAJECTORY_E2E_XIAOO_MAX_TURNS` 调整。

```bash
python3 tests/v2/regression/project_subagent_trajectory/run_e2e.py
```
