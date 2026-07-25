# Probe Codex LLM

运行：

```bash
sudo -E python3 tests/v2/regression/probe_codex_llm/run_e2e.py
```

1. 清理环境

   执行 `actraild init -f → actraild stop → actrailctl clean → actraild start`。

2. 生成随机标志 A

3. 运行 Codex

   通过双层 `actrailctl launch` 执行 Codex，并要求直接回答 A。

4. 验证回答

   Codex 标准输出必须包含 A。

5. 验证 trace

   选择内层 Codex trace，并要求正常结束。

6. 验证 LLM 采集

   `llm.request` 和 `llm.response` 均存在、数量一致，并通过 `llm.call` 一一配对。

7. 验证采集内容

   采集到的 request 和 response 中必须包含 A。
