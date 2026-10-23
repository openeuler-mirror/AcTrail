状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/core/semantic_action_runtime/src/llm_pipeline/transport/http1/ # 解析 HTTP/1.x 消息边界，并以正文容量约束执行增量内容解码。
├── [x] decoder.rs # HTTP 消息边界与 transfer framing；通过 factory 创建内容解码句柄
└── [x] content_decoder/ # HTTP/1.x 请求与响应共用；外部只访问模块入口
    ├── [x] mod.rs # 导出：DecoderFactory、DecoderHandle
    ├── [x] decoder_factory.rs # 按 Content-Encoding 选择解码实现。
    │   └── [x] struct DecoderFactory # create(headers, max_bytes) → DecoderHandle；选择一个实现
    ├── [x] decoder_handle.rs # 将单条消息的正文缓冲区与选定解码器组合。
    │   └── [x] struct DecoderHandle # append、finish、snapshot；隐藏实现和所有权方式
    ├── [x] decoder.rs # 定义增量输入、结束和正文快照接口。
    │   └── [x] trait Decoder # 有状态实现的私有增量接口
    ├── [x] bounded_body.rs # 限制累计解码正文大小，保留可输出的正文片段。
    │   └── [x] struct BoundedBody # 保存累计解码正文；执行 HTTP assembly 正文容量约束
    ├── [x] constant.rs # 缺少 Content-Encoding 时使用的编码常量
    └── [x] implementations/ # 私有实现，仅由 factory 选择
        ├── [x] mod.rs # 内部导出具体类型
        ├── [x] identity.rs # 直接保留未压缩正文。
        │   └── [x] struct IdentityDecoder # 全局无状态单例；正文由消息句柄持有
        ├── [x] brotli/ # 管理每条消息独立的 Brotli 解压状态。
        │   ├── [x] mod.rs # 内部导出 BrotliDecoder
        │   ├── [x] decoder.rs # 将增量压缩输入交给 Brotli 库并收集正文。
        │   │   └── [x] struct BrotliDecoder implements Decoder # 每消息一个 Brotli 库对象
        │   └── [x] constant.rs # Brotli 工作缓冲区参数
        ├── [x] gzip/ # 管理每条消息独立的 gzip 解压状态。
        │   ├── [x] mod.rs # 内部导出 GzipDecoder
        │   └── [x] decoder.rs # 使用 flate2 处理 gzip 正文和流结束。
        │       └── [x] struct GzipDecoder implements Decoder # 每消息一个 flate2 库对象
        └── [x] zstd/ # 管理每条消息独立的 zstd 解压状态。
            ├── [x] mod.rs # 内部导出 ZstdDecoder
            └── [x] decoder.rs # 驱动 zstd 原始解码器并接收解压正文。
                └── [x] struct ZstdDecoder implements Decoder # 每消息一个 raw Decoder 和 zio Writer
```
