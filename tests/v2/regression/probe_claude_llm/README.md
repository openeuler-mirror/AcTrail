# Probe Claude LLM

运行：

```bash
sudo -E python3 tests/v2/regression/probe_claude_llm/run_e2e.py
```

1. 清理环境

   执行 `actraild init -f → actraild stop → actrailctl clean → actraild start`。

2. 生成随机标志 A

3. 运行 Claude

   通过单层 `actrailctl launch` 执行 Claude，并要求直接回答 A。

4. 验证回答

   Claude 标准输出必须包含 A。

5. 验证 trace

   Claude trace 必须正常结束。

6. 验证 LLM 采集

   `llm.request` 和 `llm.response` 均存在、数量一致，并通过 `llm.call` 一一配对。

7. 验证采集内容

   采集到的 request 和 response 中必须包含 A。
