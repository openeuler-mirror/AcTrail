状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/core/config_export_filter/ # 选择交付配置与 Web 编辑字段；调用方提供所属模块模板并执行领域配置校验。
├── [x] Cargo.toml # 声明 serde 与保留键顺序的 TOML 依赖。
├── [x] allowlist.toml # 编译内嵌的选择策略，定义默认模式及独立校验的白名单、黑名单。
├── [x] build.rs # 内嵌策略文本或读取错误，跟踪策略文件及目录变更。
│   └── [x] struct PolicyBundle # 将读取结果写入 OUT_DIR/policy.rs，策略缺失时仍允许构建。
└── [x] src/ # 策略校验、选定字段查询与配置投影。
    ├── [x] lib.rs # 导出 ConfigExportFilter。
    └── [x] filter.rs # 数组整体选择，按输入文档顺序投影叶子字段。
        └── [x] struct ConfigExportFilter # 保存当前模式及唯一的不可变选定路径集合。
            ├── [x] bundled # 读取内嵌策略并以所属模块模板校验两份名单，明确返回缺失、非法或未知路径错误。
            ├── [x] project # 复制选中叶子；白名单裁剪空父表，黑名单保留空表。
            ├── [x] paths # 按路径顺序迭代选定字段，供调用方构建编辑字段目录。
            └── [x] allows # 查询字段是否属于当前选定集合。
```
