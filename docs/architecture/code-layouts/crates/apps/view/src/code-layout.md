状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/apps/view/src/ # 解析查看命令，读取 trace 或资源数据库，生成终端输出和导出文件。
├── [x] lib.rs # 导出命令解析与存储视图入口。
├── [x] bin/ # 终端查看器进程入口。
│   └── [x] actrailviewer.rs # 解析命令并输出查询结果或错误。
├── [x] command.rs # 校验查询、过滤、输出格式和导出参数。
│   ├── [x] struct ViewInvocation # 承载数据源、查询命令、trace 范围及输出选项。
│   ├── [x] enum StorageCommand # 区分 trace、事件、动作、负载、资源和导出查询。
│   ├── [x] enum RowLimit # 表达从头或从尾截取的结果数量。
│   ├── [x] enum OutputFormat # 选择表格或 JSON 输出。
│   └── [x] enum PayloadFormat # 选择单段负载的内容表示。
├── [x] table.rs # 计算列宽并生成终端表格。
│   └── [x] struct Table # 拥有表头和字符串行，提供 push 与 render。
├── [x] storage.rs # 按命令选择存储读取、资源查询或文件导出路径。
├── [x] storage/ # 组织 trace 数据读取、结果表示与导出文件写入。
│   ├── [x] source.rs # 解析存储配置和 trace 目标，读取快照、动作与负载。
│   ├── [x] export_json.rs # 将选定 trace 导出为 JSON 文件。
│   ├── [x] export_otel.rs # 将选定 trace 转换为 OpenTelemetry 导出数据。
│   ├── [x] export_file.rs # 处理导出目标文件的创建与写入。
│   ├── [x] render.rs # 生成 trace、进程、事件、动作和诊断表格。
│   └── [x] render/ # 按数据类型组织负载、网络及 JSON 表示。
│       ├── [x] payload.rs # 将负载字节转换为指定终端表示。
│       ├── [x] json.rs # 将查询记录序列化为 JSON。
│       └── [x] network.rs # 将网络事件关联为连接视图。
│           └── [x] struct NetworkRow # 保存单条网络展示行及关联信息。
├── [x] host_resources.rs # 定义宿主查询参数并将宿主样本输出为表格或 JSON。
│   ├── [x] struct HostResourceArgs # 宿主样本查询参数与查询范围校验。
│   └── [x] struct HostResourceView # 将宿主资源样本渲染为表格或 JSON。
├── [x] resource_source.rs # 将 Viewer 配置和查询参数交给资源数据库 Factory 与接口。
│   └── [x] struct ResourceSource # 将 Viewer 配置和查询参数交给资源数据库 Factory 与接口。
└── [x] workload_resources.rs # 将工作负载资源样本输出为表格或 JSON。
    └── [x] struct WorkloadResourceView # 渲染工作负载指标与其 trace、进程身份和采样时间。
```
