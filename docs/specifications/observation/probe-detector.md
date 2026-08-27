# 探针检测框架

> 本文定义 TLS 探针检测器的递归发现、候选选择和故障隔离行为。

Status: Accepted
Owner: `tls_probe_point_finder`
Scope: TLS 明文探针的递归发现与 candidate 选择

## 契约

检测器读取目标 ELF（Executable and Linkable Format）及相关元数据，并明确返回以下结果之一：不适用、未匹配、完整匹配、歧义或检测错误。成功结果包含 candidate、完整 evidence 和可执行的 **probe closure**；probe closure 是同时观测 outbound 与 inbound 明文所需的全部挂载点。检测器只发现挂载点，不负责安装或执行探针。

每个节点必须实现同一个递归抽象。节点既可直接完成检测，也可委托子节点；框架不得假定 provider、架构和版本具有固定层数。provider、来源、架构、运行时版本、编译器形态和具体 evidence 都只是可能的层级。

Candidate、evidence 与 consumer capability 相互独立：

- candidate 描述明文挂载点及必需的双向观测闭包；
- evidence 解释匹配原因，不依赖可变的检测器全局状态；
- consumer capability 表示某个运行时能否执行该计划。

只有同时包含加密前 outbound 明文和解密后 inbound 明文的可信观测闭包，计划才算完整。即使检测到 candidate，能力不足的 consumer 也必须拒绝执行。

## 选择与隔离

选择策略必须是显式检测节点，包括 first complete match、unique match、unique normalized closure、collect all 和 select applicable。歧义必须保留；发现顺序不得在多个冲突的完整 candidate 中静默择一。

不适用的架构或 provider 分支必须在昂贵扫描前停止。leaf 失败必须保留 detector path 和 evidence，且不得抹去诊断集合中的 sibling 结果。fast resolution 只有遇到请求方 consumer 可执行的完整 candidate 时才可短路。

配置归所属检测器子树所有。match limit 和容量上限必须在启动时校验。运行期检测错误作为 outcome 返回，不得使无关 provider 分支崩溃。

## 扩展规则

新增 provider 或 code-generation candidate 时，必须在对应检测子树下加入对象、稳定 detector identity、有界配置、evidence 类型、closure 校验和 consumer capability。不得在 fast 或 diagnostic projector 中增加第二套 provider 检测逻辑。

公共入口只负责把私有递归 outcome 投影为可执行计划或诊断报告，不得执行 provider 检测。
