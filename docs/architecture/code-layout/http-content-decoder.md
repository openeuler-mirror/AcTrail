# HTTP 内容解码代码布局

`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或待迁移走。

```text
[x] crates/core/semantic_action_runtime/src/llm_pipeline/transport/http1/
    ├── [x] decoder.rs — HTTP 消息边界与 transfer framing；通过 factory 创建内容解码句柄
    └── [x] content_decoder/ — HTTP/1.x 请求与响应共用；外部只访问模块入口
        ├── [x] mod.rs — 导出：DecoderFactory、DecoderHandle
        ├── [x] decoder_factory.rs
        │   └── [x] struct DecoderFactory — create(headers, max_bytes) → DecoderHandle；选择一个实现
        ├── [x] decoder_handle.rs
        │   └── [x] struct DecoderHandle — append、finish、snapshot；隐藏实现和所有权方式
        ├── [x] decoder.rs
        │   └── [x] trait Decoder — 有状态实现的私有增量接口
        ├── [x] bounded_body.rs
        │   └── [x] struct BoundedBody — 保存累计解码正文；执行 HTTP assembly 正文容量约束
        ├── [x] constant.rs — 缺少 Content-Encoding 时使用的编码常量
        └── [x] implementations/ — 私有实现，仅由 factory 选择
            ├── [x] mod.rs — 内部导出具体类型
            ├── [x] identity.rs
            │   └── [x] struct IdentityDecoder — 全局无状态单例；正文由消息句柄持有
            ├── [x] brotli/
            │   ├── [x] mod.rs — 内部导出 BrotliDecoder
            │   ├── [x] decoder.rs
            │   │   └── [x] struct BrotliDecoder implements Decoder — 每消息一个 Brotli 库对象
            │   └── [x] constant.rs — Brotli 工作缓冲区参数
            ├── [x] gzip/
            │   ├── [x] mod.rs — 内部导出 GzipDecoder
            │   └── [x] decoder.rs
            │       └── [x] struct GzipDecoder implements Decoder — 每消息一个 flate2 库对象
            └── [x] zstd/
                ├── [x] mod.rs — 内部导出 ZstdDecoder
                └── [x] decoder.rs
                    └── [x] struct ZstdDecoder implements Decoder — 每消息一个 raw Decoder 和 zio Writer
```

## 数据与生命周期

HTTP framing 处理后的输入切片交给一个内容解码实现，再将解压后的正文交给
SSE/JSON 投影。消息持有一个句柄；gzip、Brotli、zstd 的库状态随消息创建和释放。
identity 单例执行透传，正文由消息句柄持有。

库的历史窗口、上下文和临时输出缓冲区由库管理，累计解码正文由一个
`BoundedBody` 持有。输入直接借用。正文容量上限约束累计输出，库内部工作缓冲另有内存开销。
快照共享 `Arc`，调用链同步消费后释放。
清理 identity 正文时，独占缓冲区直接清空；仍被快照持有时替换为空缓冲区。

## 库调用与失败

gzip 委托 `flate2::write::MultiGzDecoder`；Brotli 委托
`brotli::DecompressorWriter`；zstd 委托
`zstd::stream::zio::Writer<BoundedBody, zstd::stream::raw::Decoder>`。
追加和结束均使用库原生接口；gzip、zstd 每次追加后调用 flush，
将库已产生的输出交给正文存储。zstd 结束时调用 Writer::finish 校验帧完整性。
解压校验由库负责，库原生错误沿消息失败通道返回。
正文容量限制由 `BoundedBody` 承担。

各压缩实现使用库原生窗口设置。
`semantic_retention.l0_llm_call.assembly.max_buffer_bytes` 约束累计解码正文，
库的历史窗口、上下文和临时输出缓冲区另有内存开销。

本模块处理 HTTP/1.x 消息的单个 Content-Encoding。

## 常量

| 所属模块 | 常量 | 值 | 含义 |
| --- | --- | --- | --- |
| content_decoder | `DEFAULT_ENCODING_IF_CONTENT_ENCODING_MISSING` | `"identity"` | 缺少 Content-Encoding 时使用透传 |
| implementations/brotli | `BUFFER_SIZE` | `0` | 委托 Brotli 库选择标准工作缓冲区 |

identity 编码常量与 Brotli 缓冲区参数定义在各自模块的 `constant.rs`。
gzip、zstd 使用库原生构造接口。
