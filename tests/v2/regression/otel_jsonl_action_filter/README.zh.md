# OTEL JSONL 动作筛选回归

本用例只验证 builtin `otel-jsonl` 插件的动作筛选能力：

- Web API 和 Schema 暴露可编辑的 action kind 勾选项；
- 每轮只导出已启用的 action kind；
- 未启用的 action kind 不进入 JSONL；
- 插件运行结束时保持 active 且 `dropped_records=0`。

用例通过真实 Agent 产生代表性动作，但不验证 semantic action 的终态、
持久化、identity、exit 或 exec 配对规则。这些行为由
`semantic_action_boundaries` 独立验证。

```bash
sudo -E python3 tests/v2/regression/otel_jsonl_action_filter/run_e2e.py \
  --cleanup
```
