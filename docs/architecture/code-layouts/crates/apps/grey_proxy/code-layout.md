状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/apps/grey_proxy/ # WASM grey 运行时的子进程，与所属运行时同生同死；补充近期进程动作并请求外部裁决
├── [x] Cargo.toml # 子进程二进制入口；复用现有存储查询与 HTTP 能力
└── [x] src/ # 待决动作来自 WASM 请求，逻辑进程 ID 来自 daemon，历史事实来自现有数据库
    ├── [x] bin/ # 子进程入口
    │   └── [x] actraild-grey-proxy.rs # 接收运行时提供的配置，启动请求处理，并随所属运行时退出
    ├── [x] config.rs # 外部裁决服务地址、历史时间窗口和数量上限；存储位置及已有运行限制由宿主配置提供
    │   └── [x] struct Config # 解析并持有 proxy 所需配置，整体传入请求处理对象
    ├── [x] proxy.rs # 接收待决请求，补充历史，调用外部服务并返回结果
    │   └── [x] struct GreyProxy # 组织单次处理；历史不可用时注明原因，裁决调用失败交回现有 grey 故障处理
    ├── [x] history.rs # 倒序选取有界历史 ID，再批量读取选中记录的属性
    │   └── [x] struct HistoryReader # 按 trace、逻辑进程 ID 和时间窗口提取访问裁决及执行命令，输出精简事实
    └── [x] remote.rs # 外部裁决服务调用
        └── [x] struct DecisionClient # 发送待决动作与历史上下文，返回外部裁决；沿用现有调用预算和资源限制
```
