# V2 测试执行输入与清理生命周期

## 目标

统一单 case 与 `test_all.py` 的输入、workspace 和清理语义，避免 case 或 task
自行决定临时路径、擅自保留证据，或者在聚合执行时提前销毁外层仍需管理的状态。

## 执行顺序

```text
runner 解析公共参数
    │
    ├── 为 definition 计算独立 work_dir
    ├── 构造 TestCaseInputs
    ├── build_case(inputs)
    ├── case.run(context)
    ├── 可选调用 case.cleanup(context)
    └── 可选删除 case workspace 和 runner log
```

`task.run()` 只执行测试动作和断言，不负责最终清理。

## TestCaseInputs

`tests/v2/common/config.py` 中的 `TestCaseInputs` 是 runner 传给 case factory 的
唯一公共输入对象：

```python
@dataclass(frozen=True)
class TestCaseInputs:
    repo: Path
    bin_dir: Path
    work_dir: Path
```

约束如下：

- `repo`、`bin_dir` 和 `work_dir` 均由 runner 确定；
- `TestDefinition.build_case` 的签名必须是
  `Callable[[TestCaseInputs], TestCase]`；
- case、task 和 case 专属 environment 不得重新解析或覆盖 `work_dir`；
- case 专属配置工厂可以读取超时、model 等专属环境变量，但必须接收
  `TestCaseInputs`，并原样传递其中的公共路径；
- 新增公共执行输入时优先扩展 `TestCaseInputs`，禁止继续增加
  `build_case` 的位置参数。

## Workspace 所有权

workspace 的创建和删除只属于 `tests/v2/common/runner/`：

- 默认根目录是 `/tmp/actrail-regression`；
- 每个 definition 的默认目录是 `<work-root>/<definition.name>`；
- runner 在构造 case 前准备空目录；
- case 只能在收到的 `work_dir` 中写入日志以外的临时文件、导出文件和证据；
- case 不得删除 `work_dir` 或其父目录；
- runner 只允许递归删除 `work_root` 的直接子目录，禁止把 `/`、用户 home 或仓库
  根目录用作 `work_root`；
- 空的 `work_root` 可以由 runner 使用非递归 `rmdir` 收拢。

插件导出路径、Web 日志和配置快照等 case-owned 文件必须指向 `work_dir`，不得写入
另一个临时根目录。需要修改系统或用户配置的测试必须在 cleanup hook 中恢复原值。

## 清理控制

公共 CLI 提供：

```text
--cleanup
--no-cleanup
```

语义如下：

| 模式 | case cleanup hook | workspace | runner log |
| --- | --- | --- | --- |
| `--cleanup` | 调用 | 删除 | 删除 |
| `--no-cleanup` | 不调用 | 保留 | 保留 |

`--cleanup` 是默认值。单 case 的 `run_e2e.py` 和聚合入口
`tests/v2/regression/test_all.py` 使用同一参数：

- 单 case 调试时，调用者可以使用 `--no-cleanup` 保留现场；
- `test_all.py` 由外层统一决定是否清理；
- task 不得根据自己是否单跑来猜测清理策略；
- `SKIPPED` case 也遵守外层清理策略；
- cleanup hook 失败必须使 case 最终结果为 `FAILED`；
- workspace 或 runner log 删除失败同样属于 `FAILED`。

已有 case 如果仍在 `run()` 的 `finally` 中执行清理，应逐步迁移到 cleanup hook；
在迁移完成前，其内部清理行为不受 `--no-cleanup` 控制，不能作为新 case 的参考。

## 职责边界

| 组件 | 负责 | 禁止 |
| --- | --- | --- |
| `runner.py` | 公共 CLI、inputs、workspace、cleanup 调用、日志和汇总 | 插件或 workload 专属判断 |
| `TestCase` | 编排外部条件、environment 和 task，汇总结果 | 自行解析公共路径 |
| `TestCase.cleanup()` | 恢复外部状态、停止服务、卸载插件、清理 trace | 删除 runner 管理的 workspace |
| `task.py` | 配置被测对象、运行 workload、轮询和断言 | 最终清理、公共参数解析 |
| case environment | 管理该类被测环境的生命周期 | 决定 workspace 路径 |
| `test_all.py` | 注册、筛选并把公共策略交给 runner | 复制 case 内部实现 |

## 可复用能力

跨 case 能复用的能力必须进入 `tests/v2/common/`。当前公共边界包括：

- `agent_selection.py`：候选 Agent 顺序、binary 发现和参数映射；
- `plugin_web_api.py`：通过 `curl` 调用 actrailweb 插件控制 API；
- `plugin_test_environment.py`：daemon、Web、插件加载、原配置恢复和卸载；
- `actrail_runtime.py`：AcTrail binaries、命令执行和 daemon 生命周期；
- `runner.py`：TestDefinition 执行和外层清理；
- `test_case/`：case/result/status contract。

插件 Schema、action kind 组合、特定 OTEL 字段和业务断言必须保留在对应 case
目录，不得下沉到公共层。
