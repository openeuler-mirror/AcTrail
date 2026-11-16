# V2 tests

V2 测试结果遵循以下统一判定原则：

- `SKIPPED`：外部测试条件不满足，测试无法有效执行。例如可选 agent
  二进制缺失或不可执行、agent 未认证、provider/model 不可用，或者外部网络不可达。
- `PASSED`：外部条件满足，测试完整执行，并且所有 AcTrail 行为和采集断言均通过。
- `FAILED`：测试已经具备执行条件，但 AcTrail 启动、运行、采集、输出或断言失败；
  测试代码自身异常也属于失败。

AcTrail 自身是被测对象，不属于可选外部条件。`actraild`、`actrailctl`、
`actrailviewer` 或其他测试要求的 AcTrail release 产物缺失时必须判定为
`FAILED`，不能降级为 `SKIPPED`。

当前 V2 regression 测试的运行方法见
[`regression/README.md`](regression/README.md)。
