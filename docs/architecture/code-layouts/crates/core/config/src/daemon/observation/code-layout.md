状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/core/config/src/daemon/observation/ # 配置文件活动的采集、汇总和内容保留。
└── [x] file.rs # 定义文件观测开关、默认值、保留枚举及终端路径匹配。
    ├── [x] struct FileObservationConfig # 组合文件采集、汇总、元数据、终端、批量读取和目录枚举配置。
    ├── [x] struct FileIoSummaryConfig # 设置汇总刷新周期及文件、对象条目容量。
    ├── [x] struct FileCollectionConfig # 选择打开、路径变更、描述符变更及读写采集项。
    ├── [x] struct FileIoCollectionConfig # 设置读写观测、次数、字节数和错误计数。
    │   └── [x] enabled # 判断是否启用了任一读写采集项。
    ├── [x] enum FileMetadataRetention # 选择完整或精简元数据。
    ├── [x] enum FileRawEventRetention # 选择完整原始事件、仅错误或汇总。
    │   ├── [x] retains_success # 判断是否保留成功事件。
    │   └── [x] retains_error # 判断是否保留错误事件。
    ├── [x] enum FileBulkReadMode # 选择批量读取计数或路径集合。
    ├── [x] struct FileTtyObservationConfig # 配置终端路径、操作和原始事件保留方式。
    │   └── [x] matches # 同时匹配已启用的终端路径规则与操作名称。
    ├── [x] struct FileBulkReadObservationConfig # 配置批量读取模式、原始事件策略及路径集合容量。
    └── [x] struct FsEnumerateObservationConfig # 配置目录枚举识别阈值、原始事件策略及路径集合容量。
```
