状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/apps/web/ # 提供本地存储与集群数据的 HTTP 查询、插件管理和 Vue 可视化界面。
├── [x] Cargo.toml # Web 服务器、存储查询与嵌入式前端依赖。
├── [x] build.rs # 执行 Vite 构建并生成嵌入式静态资源表。
├── [x] src/ # 配置解析、HTTP 路由、查询投影与资源服务。
│   ├── [x] lib.rs # 导出 Web 应用入口并声明后端模块。
│   ├── [x] main.rs # 解析进程参数并启动 HTTP 服务器。
│   ├── [x] args.rs # 校验监听地址、存储位置和查询限制配置。
│   │   ├── [x] struct WebConfig # 持有存储或集群入口、监听地址、请求超时与 operator 配置。
│   │   └── [x] enum WebMode # 区分本地存储与集群展示模式。
│   ├── [x] http.rs # 校验数据源、监听 TCP 连接并分派请求处理。
│   │   ├── [x] struct WebContext # 持有连接处理共享的查询配置、告警入口和各类响应限制。
│   │   └── [x] enum RequestBudget # 选择持续服务或处理固定数量请求后退出。
│   ├── [x] http/ # 按资源和操作划分路由，校验查询预算与输入参数。
│   ├── [x] view.rs # 组织存储记录到前端 JSON 的投影入口。
│   │   └── [x] enum LlmNavMode # 表达 LLM 请求内容的导航模式。
│   ├── [x] view/ # 构造 trace、动作、统计、时间归因与集群视图。
│   ├── [x] json.rs # 提供后端 JSON 表示所需的共用转换。
│   ├── [x] render.rs # 按 URL 查找构建时嵌入的静态资源。
│   │   ├── [x] struct StaticAsset # 表达对外返回的资源内容和类型。
│   │   └── [x] struct EmbeddedAsset # 描述构建产物的嵌入式资源记录。
│   └── [x] plugins/ # 读取本机安装的插件包并生成管理界面所需元数据。
│       ├── [x] mod.rs # 声明插件目录读取与表示模块。
│       ├── [x] catalog.rs # 汇总插件包、加载参数及授权选项。
│       │   ├── [x] struct InstalledPluginCatalog # 持有插件安装目录并生成目录快照。
│       │   ├── [x] struct CatalogSnapshot # 汇总当前可展示的安装包及目录诊断。
│       │   └── [x] struct PluginLoadOptions # 描述插件加载表单所需的配置和授权选项。
│       ├── [x] package.rs # 从文件系统读取安装包及 manifest。
│       │   ├── [x] struct PluginDirectory # 表达插件安装目录与包发现入口。
│       │   └── [x] struct InstalledPackage # 保存单个安装包的位置和读取结果。
│       └── [x] render.rs # 将插件目录结果转换为 HTTP 响应数据。
└── [x] frontend/ # Vue 工作区、trace 视图和共享交互组件。
```
