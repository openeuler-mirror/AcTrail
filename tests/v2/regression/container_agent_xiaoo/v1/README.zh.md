# 真实 xiaoO 双容器 V2 回归

该测例通过 V2 公共 runner 调用已有的
`tests/agent-trace/multi-container-xiaoo/run_e2e.py`。它在两个独立 Docker
容器内运行真实 xiaoO 二进制，并验证：

- 两条 trace 同时保持 Active；
- 每条 trace 都有独立的 eBPF 进程和网络证据；
- 文件读写 action 只归属于对应任务；
- TLS/socket payload 和 `llm.call`、`llm.request`、`llm.response` 完整；
- 请求和响应标记不会串到另一个容器的 trace。

xiaoO 使用仓库启动的本地 OpenAI-compatible 流式服务，因此不需要真实 API
Key，也不验证外部模型供应商的认证和可用性。

## 运行

```bash
sudo -E \
  CONTAINER_AGENT_XIAOO_BINARY=/path/to/xiaoo \
  CONTAINER_AGENT_XIAOO_IMAGE=ubuntu:24.04 \
  python3 tests/v2/regression/test_all.py \
    --case container_agent_xiaoo \
    --no-cleanup
```

环境变量：

- `CONTAINER_AGENT_XIAOO_BINARY`：真实 xiaoO 可执行文件。
- `CONTAINER_AGENT_XIAOO_IMAGE`：匹配宿主架构的运行镜像，默认
  `ubuntu:24.04`。
- `CONTAINER_AGENT_XIAOO_E2E_TIMEOUT_SECONDS`：完整验收超时，默认 900 秒。
- `CONTAINER_AGENT_XIAOO_E2E_CLEANUP_GRACE_SECONDS`：超时后清理宽限，
  默认 30 秒。
