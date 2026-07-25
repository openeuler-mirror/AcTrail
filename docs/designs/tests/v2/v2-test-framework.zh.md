# V2 测试准则

## 目标

`tests/v2/` 用于可独立运行、可汇总执行、可由人手动复现的 AcTrail 测试。

## 结果判定

| 状态 | 判定 |
| --- | --- |
| `SKIPPED` | 外部测试条件不满足，无法有效执行测试 |
| `PASSED` | 外部条件满足，测试完整执行，所有 AcTrail 断言通过 |
| `FAILED` | 测试具备执行条件，但 AcTrail、测试断言或测试代码失败 |

外部测试条件包括可选 agent、浏览器、容器 runtime、登录状态、provider/model
和外部网络。

AcTrail 自身不属于外部条件。所需 AcTrail binaries 缺失、daemon 启停失败、
trace/action/payload 缺失或断言失败，必须是 `FAILED`。

外部条件检查应在清理存储和启动 daemon 前完成。外部预检已经成功后，AcTrail
包裹运行或采集失败应判为 `FAILED`，不能改成 `SKIPPED`。

## 通用准则

- 每个测试只验证一个明确目标。
- 测试必须使用本次运行唯一的 marker 关联 workload 输出和 AcTrail 证据。
- 等待异步结果必须有明确上限。
- 测试必须清理自己启动的 daemon 和临时资源。
- 汇总测试中，一个 case 失败不应阻止后续 case 运行。
- 日志不得包含 API key、token、authorization header 或其他凭据。
- 真实 agent 测试应关闭 session 持久化；不需要工具时应禁用工具。
- 不同测试共享的 runner、runtime、状态和断言逻辑放入 `tests/v2/common/`。
- 只属于一个测试的命令、配置和判断保留在该测试目录。

## Regression 文档

每个 `tests/v2/regression/<case>/` 必须包含 `README.zh.md`。文档必须让不了解
测试实现的人可以从仓库根目录逐步复现，格式见
[V2 Regression 文档规范](regression-authoring.zh.md)。
