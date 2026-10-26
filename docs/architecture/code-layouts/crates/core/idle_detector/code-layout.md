状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/core/idle_detector/ # 读取 agent 的执行状态，按持续时间产生空闲告警与恢复事件。
└── [x] src/ # 检测配置、空闲片段状态及告警输出。
    ├── [x] lib.rs # 导出 IdleDetector、配置和告警类型。
    ├── [x] config.rs # 定义检测开关、轮询周期与空闲阈值。
    │   └── [x] struct IdleDetectionConfig # 提供默认值并校验检测时间参数。
    ├── [x] runtime.rs # 拥有轮询时钟及每个执行实例的空闲片段；执行事实来自 ExecutionStates。
    │   ├── [x] struct IdleDetector # 依据权威执行状态计算空闲与恢复，不独立维护执行状态。
    │   │   ├── [x] poll # 到期后扫描执行状态，输出本轮告警；未知状态不作为恢复依据。
    │   │   └── [x] finish_trace # 清理已结束 trace 的空闲片段。
    │   └── [x] struct Episode # 保存空闲起点的单调时刻和事件墙钟时间。
    └── [x] alert.rs # 将检测结果转换为告警契约中的事件。
        └── [x] struct IdleAlert # 关联 trace 和告警草稿；草稿携带执行实例、空闲时刻、阈值及恢复事件种类。
```
