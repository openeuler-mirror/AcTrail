# Probe Detector 设计文档

本目录收录 AcTrail TLS 明文 Probe Detector 的设计规范和当前实现索引。该模块负责定位目标进程发出 HTTPS/TLS 请求前和收到 HTTPS/TLS 响应后的明文读写入口，为后续 HTTP、WebSocket 和 LLM request/response 观测提供原始数据。

## 阅读顺序

1. [Probe Detector 递归检测框架规范](probe-detector.md)
   - 解释 probe 与 detector 的用途。
   - 定义统一递归 `ProbeDetector`、检测结果、聚合策略、架构隔离、candidate evidence 和 consumer capability。
   - 作为后续 Probe Detector 编码与评审的规范性参考。

2. [Probe Detector 当前实现路径地图](current-implementation-paths.zh.md)
   - 标识当前源码、consumer 和真实 E2E 文件路径。
   - 说明迁移后的单一 detector 实现路径和 fast/detect 投影职责。
   - 记录受主机架构或工具链限制的待验证范围。

3. [Probe Detector 目标文件路径](targeted-file-paths.zh.md)
   - 定义重构后的递归目录与 namespace。
   - 分层放置 contract、provider、architecture 和 signature/codegen candidate。
   - 明确版本验证信息与 detector 路径分离。

## 文档职责

| 文档 | 性质 | 更新时机 |
| --- | --- | --- |
| `probe-detector.md` | 规范性目标设计 | 接口、结果语义、架构原则或评审规则变化时 |
| `current-implementation-paths.zh.md` | 当前代码路径快照 | 模块移动、实现迁移、provider/candidate/consumer/E2E 变化时 |
| `targeted-file-paths.zh.md` | 目标源码路径与 namespace 规范 | 目标模块边界、contract 分层或 detector 递归结构变化时 |
| `README.zh.md` | 目录索引 | 新增、删除或重命名本目录文档时 |

当前实现不能仅因为被记录在路径地图中就获得规范地位；发生冲突时，应先依据目标规范判断是实现需要迁移还是规范需要显式修订。
