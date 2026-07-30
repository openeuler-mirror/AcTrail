# Semantic action 边界回归

本用例验证 semantic action 自身的终态和边界行为，不验证 OTEL 插件的
Schema、Web 勾选组合或筛选矩阵。

真实 Agent 场景验证：

- 同一逻辑进程中的 bash 和 Agent exec 分别形成一次终态动作；
- 多次 LLM request 只建立一个 Agent identity；
- `process.exit` 和 `agent.exit` 只在线导出，不进入 action storage；
- 根进程在线导出的 action ID 非空且每个只出现一次；
- storage 中保留预期的持久化动作，不存储 export-only exit 动作；
- exec intent 和 completion 正确配对。

另外运行 seccomp-only 失败、eBPF-only completion 和非零退出三个真实命令边界。
OTEL JSONL 在本用例中只是在线导出结果的观测通道；持久化行为由
`actrailviewer` 独立查询。

```bash
sudo -E python3 tests/v2/regression/semantic_action_boundaries/run_e2e.py \
  --cleanup
```
