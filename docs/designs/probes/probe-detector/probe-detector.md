# Probe Detector 递归检测框架规范

## 背景

AcTrail 需要在不修改被观测程序业务逻辑的前提下，定位 TLS 实现中的明文读写边界，从而采集目标进程发出的 HTTPS/TLS 请求数据和收到的 HTTPS/TLS 响应数据。当前 Probe Detector 的直接用途是为 OpenSSL、BoringSSL、Rustls、Go crypto/tls、GnuTLS 和 NSS/NSPR 等 TLS provider 找到这些明文入口，而不是泛指任意运行时数据采集。

本文中的 **probe** 是 TLS 明文采集挂载点及其采集语义：它描述应该在目标程序或共享库的哪个 TLS 函数位置挂载、采集 outbound request 还是 inbound response、在函数入口还是返回点读取，以及应采用哪种参数或返回值解释方式。Probe point 可以由 eBPF uprobe、同步 runtime inline hook 或其他 consumer 执行，但 detector 本身不负责安装和运行 probe。

Probe 的目标是在请求被 TLS 加密前取得 outbound plaintext，并在响应完成 TLS 解密后取得 inbound plaintext。采集到的明文随后才能继续组装为 HTTP、HTTP/2、WebSocket，以及进一步识别 LLM request/response。一个可执行的 probe plan 通常不能只有单个地址，而需要形成完整观测闭包，同时具备 outbound request 和 inbound response 的可信入口。

**Detector** 是负责发现、验证和解释 probe point 的检测对象。它读取目标 ELF、架构、符号、依赖库、静态机器码或语言运行时元数据，判断某个检测策略是否适用，并产生以下结果之一：不适用、未匹配、完整匹配、歧义或检测错误。成功结果包含 probe candidate、完整证据和可供 consumer 校验的 probe plan。

不同目标可能需要完全不同的检测路径。例如：

```text
Rustls stripped executable
  → architecture
    → library/codegen candidate
      → static machine-code patterns

OpenSSL dynamically linked executable
  → DT_NEEDED library discovery
    → concrete libssl candidate
      → exported symbol closure

Go executable
  → .gopclntab
    → crypto/tls endpoint closure
```

现有 finder 已包含这些检测思路，但其控制流和结果模型仍有分散实现。本文定义一个统一、可递归组合的 Probe Detector 模型，作为后续重构、新 provider 接入、新架构支持和 candidate 维护的编码参考。本文描述的是目标设计规范，不表示当前代码已经完成全部接口迁移；当前源码位置和概念映射见同目录的《当前实现路径地图》。

## 1. 文档地位

本文是 Probe Detector 模块后续设计、实现和评审的参考规范。

本文使用以下规范词：

- **必须**：实现不得违反。
- **禁止**：实现不得采用。
- **应该**：除非有明确、可记录的工程理由，否则应遵循。
- **可以**：允许采用，但不是强制要求。

若现有实现与本文不一致，应先判断差异属于缺陷、迁移中的技术债还是本文需要修订。不得为了局部改动静默绕过本文约束。修改公共 contract 或跨模块 spec 时，仍需遵守仓库的授权要求。

## 2. 目标

Probe Detector 必须支持层次化、可递归组合的探针检测：

```text
target
  → provider
    → source
      → architecture
        → library/runtime version
          → compiler and codegen shape
            → concrete evidence detector
```

层级不是固定 schema。任意检测节点既可以直接完成检测，也可以继续委托给子检测器。框架不得假设树只有 `provider → architecture → version` 三层。

核心目标：

1. 所有检测节点通过同一个抽象参与检测。
2. 分支节点和叶子节点具有相同的调用方式。
3. 架构、provider、版本和 codegen 差异能够显式隔离。
4. 检测结果必须携带完整证据路径，不只返回裸 offset。
5. `fast` 与 `detect` 可以共享检测器，只采用不同聚合策略。
6. 新增 candidate 时以追加为主，不破坏已验证 candidate。
7. 启动时遇到歧义、损坏或不完整闭包必须 fail-fast。
8. finder 能生成 plan 与 collector 能执行 plan 必须分别表达。

## 3. 非目标

本框架不负责：

- 解析 TLS、HTTP、WebSocket 或 LLM payload。
- 加载 eBPF、安装 inline hook 或启动被观测进程。
- 把所有 provider 强行建模成相同的固定层数。
- 用宽松 fallback 掩盖未知版本或不完整证据。
- 仅凭库版本号推断机器码一定相同。

## 4. 核心抽象

所有节点必须实现统一检测接口。推荐接口形态如下：

```rust
trait ProbeDetector {
    fn id(&self) -> &DetectorId;

    fn detect(
        &self,
        context: &ProbeContext,
    ) -> Result<DetectionOutcome, DetectionError>;
}
```

每个具体 detector 必须由同 namespace 的具名 config 构造。构造配置错误使用独立的 `DetectorConfigError`，并在启动阶段 fail-fast；不得混入运行期 `DetectionError`。空配置同样必须保留具名结构体与 `try_new(config)` 边界。

接口语义：

- `id` 必须在当前 detector tree 内稳定且可读。
- `detect` 必须只读取 `ProbeContext`，不得隐式修改全局检测状态。
- 节点可以直接分析目标，也可以调用任意数量的子 `ProbeDetector`。
- 调用者不得依赖节点是分支还是叶子。

这是一种递归 Composite 模型：

```text
ProbeDetector
├─ leaf detector
└─ composite detector
   ├─ ProbeDetector
   ├─ ProbeDetector
   └─ composite detector
      └─ ProbeDetector
```

实现上的树深度可以任意扩展，但配置或外部构造的 detector tree 必须有明确边界，禁止无界递归或循环引用。

## 5. 检测上下文

`ProbeContext` 应集中持有一次检测所需的只读事实：

```rust
struct ProbeContext<'a> {
    target: &'a TargetIdentity,
    target_image: &'a ElfImage,
    probe_image: &'a ElfImage,
    request: &'a ProbeDetectionRequest,
}
```

`ProbeDetectionRequest` 表达本轮用户选择，包括 architecture/provider/source filters、consumer、显式 library paths 和搜索目录。`ProbeContext` 只保存解析后的只读事实和 request 引用。shared-library discovery detector 为每个候选库创建派生 context 后调用其 symbol child，不修改根 context。

匹配上限、聚合策略和 candidate enablement 属于对应 detector 的 config，不得放入 `ProbeContext` 或依赖全局常量。

至少需要区分：

- 目标 ELF 的实际架构。
- 用户显式指定的过滤条件引用。
- 当前正在检测的 executable 或 shared-library image。
- 最终消费 plan 的 collector/runtime。
- 诊断输出允许保留的 evidence 数量。

架构判断必须以解析后的 `image.arch()` 为事实来源。文件名、安装目录、版本字符串和调用者猜测不能替代 ELF 架构。

## 6. 检测结果

禁止用 `Option<ProbePlan>` 同时表达“不适用”“没有匹配”“发生歧义”和“检测器损坏”。推荐使用显式结果：

```rust
enum DetectionOutcome {
    Inapplicable(DetectionEvidence),
    NoMatch(DetectionEvidence),
    Matched(ProbeCandidate),
    Ambiguous(AmbiguousDetection),
    Collected(DetectionEvidence),
}
```

含义：

- `Inapplicable`：当前节点不适用于这个上下文，例如 aarch64 目标进入 x86_64 detector。
- `NoMatch`：节点适用，但证据没有满足 candidate 条件。
- `Matched`：得到一个完整且可验证的 candidate。
- `Ambiguous`：得到多个互不等价的有效 candidate，不能安全选择。
- `Collected`：聚合节点为诊断保留其直接子节点的独立 outcome；它本身不是可执行 candidate，也不表示多个命中彼此歧义。
- `Err(DetectionError)`：输入损坏、ELF 无法解析、offset 溢出或 detector 自身无法完成承诺的检查。

`NoMatch` 不是错误。`Ambiguous` 在生成启动 plan 时必须成为启动错误，禁止选择“看起来最像”的结果。

## 7. Candidate 与 Evidence

### 7.1 Candidate

`ProbeCandidate` 表示一个完整的候选检测结果：

```rust
struct ProbeCandidate {
    detector_path: DetectorPath,
    provider: TlsProvider,
    source: ProbeSource,
    architecture: Architecture,
    points: Vec<ProbePoint>,
    evidence: DetectionEvidence,
    capability: CapabilityClaim,
}
```

Candidate 必须满足其 detector 声明的完整闭包。对双向 TLS plaintext，至少同时包含：

- 一个可信的 outbound probe point。
- 一个可信的 inbound probe point。

单个孤立 pattern 命中不得提升为完整 candidate。

### 7.2 Evidence

Evidence 必须足以解释匹配或不匹配：

- detector 完整路径。
- 实际架构。
- pattern ID 或 symbol 名称。
- pattern 长度和命中数量。
- 命中的 file offset 与 virtual address。
- shared library 的路径、来源和架构。
- `BinaryIdentity` 的 identity type code 与 identity；GNU build-id 只是可选 provider，不得把其缺失解释为 identity 缺失。
- candidate 被拒绝的具体原因。

Evidence 不得只保留最终 offset，否则无法区分可靠匹配、偶然命中和错误 fallback。

## 8. Detector Path

每个结果必须保留从根节点到叶子的稳定路径。例如：

```text
tls/rustls/static-pattern/aarch64/common-state-pair-48-56
```

路径至少应回答：

- 哪个 probe domain。
- 哪个 provider。
- executable 还是 shared library。
- 哪个架构。
- 哪个 signature/codegen candidate。
- 哪个具体 detector 产生了证据。

不得只用 `resolver = rustls-symbol-map` 代替完整路径。Resolver 名称可以继续存在，但它不是完整 candidate 身份。

## 9. 分支与聚合策略

分支节点必须显式声明如何聚合子结果：

```rust
enum SelectionPolicy {
    FirstComplete,
    UniqueMatch,
    UniqueClosure,
    CollectAll,
    SelectApplicable,
}
```

### 9.1 `FirstComplete`

按确定顺序选择第一个完整 candidate，适用于启动敏感的 `fast` 路径。

要求：

- 顺序必须有文档。
- 子 detector 的真实错误不得被当作 `NoMatch` 吞掉。
- 只允许完整 candidate 提前结束。

### 9.2 `UniqueMatch`

要求恰好一个子 detector 匹配。零个返回 `NoMatch`，多个返回 `Ambiguous`。

### 9.3 `UniqueClosure`

允许多个 detector 命中，但它们必须解析到同一组规范化 probe points。若得到不同闭包，返回 `Ambiguous`。

Rustls 同架构下的版本化 static candidates 应采用这一策略。

### 9.4 `CollectAll`

收集当前聚合节点的全部直接子 outcome，返回 `Collected`，适用于诊断命令。它不得改变各 candidate 自身的成立条件，也不会递归覆盖后代节点自己的 selection policy。

若某个诊断模式承诺完整 evidence tree，就必须为需要展开的每一层显式配置诊断策略；只在根节点使用 `CollectAll` 仅能保证收集全部根子节点。

### 9.5 `SelectApplicable`

根据上下文中的确定事实只选择一个适用分支，例如按 `image.arch()` 选择架构 detector。未被选择的架构分支应标记为 `Inapplicable`，不得执行机器码扫描。

## 10. 架构隔离

架构是 detector 身份的一部分。机器码 pattern 必须位于架构专属 detector 下：

```text
RustlsStaticDetector
├─ X86_64RustlsDetector
│  ├─ CommonStatePair2732Detector
│  └─ CommonStatePair4132Detector
└─ Aarch64RustlsDetector
   ├─ CommonStatePair5264Detector
   └─ CommonStatePair4856Detector

OpenSSLStaticDetector
├─ X86_64OpenSSLDetector
│  └─ Codex146SslExPairDetector
└─ Aarch64OpenSSLDetector
   └─ SslExEntryPair3232Detector
```

必须满足：

1. x86_64 pattern 禁止扫描 aarch64 ELF。
2. aarch64 pattern 禁止扫描 x86_64 ELF。
3. shared library 架构必须与 target 架构一致。
4. plan 必须记录 probe binary 和 target 的实际架构。
5. collector 必须再次验证自身能够执行该架构的 plan。

符号表、demangle 或 pclntab 等解析算法可以复用通用实现，但“算法通用”不得自动宣称“所有架构均已验证”。

Provider 的路径必须显式包含 `x86_64` 与 `aarch64` namespace。若两个架构的 detector 行为、闭包要求、ABI 解释和 evidence 构造完全相同，共享实现应放入 `common/`，两个架构模块通过 `mod.rs` re-export；不得为了减少目录而隐藏架构能力边界。

## 11. Provider 组合示例

Rustls detector 可以按以下方式组合：

```text
RustlsDetector : ProbeDetector
├─ RustlsDemangledSymbolDetector : ProbeDetector
└─ RustlsStaticDetector : ProbeDetector
   ├─ X86_64RustlsDetector : ProbeDetector
   │  ├─ CommonStatePair2732Detector : ProbeDetector
   │  └─ CommonStatePair4132Detector : ProbeDetector
   └─ Aarch64RustlsDetector : ProbeDetector
      ├─ CommonStatePair5264Detector : ProbeDetector
      └─ CommonStatePair4856Detector : ProbeDetector
```

OpenSSL detector 可以按以下方式组合，static candidate 同样按机器码形态命名：

```text
OpenSSLDetector : ProbeDetector
├─ OpenSSLSharedLibraryDetector : ProbeDetector
└─ OpenSSLExecutableDetector : ProbeDetector
   ├─ X86_64OpenSSLDetector : ProbeDetector
   │  └─ Codex146SslExPairDetector : ProbeDetector
   └─ Aarch64OpenSSLDetector : ProbeDetector
      └─ SslExEntryPair3232Detector : ProbeDetector
```

未来可以继续细分而不修改统一接口：

```text
CommonStatePair4856Detector
├─ Rustc190Detector
│  ├─ ThinLtoDetector
│  │  ├─ CodegenShape1Detector
│  │  └─ CodegenShape2Detector
│  └─ FullLtoDetector
└─ Rustc191Detector
   └─ ThinLtoDetector
```

OpenSSL、BoringSSL、Go、GnuTLS 和 NSS 可以采用不同的内部树，但根节点仍实现 `ProbeDetector`。

## 12. 版本与 Codegen Candidate

机器码 candidate 的身份禁止只依赖库版本。最终机器码还可能受以下因素影响：

- CPU architecture。
- rustc/clang/gcc 版本。
- enabled features。
- optimization level。
- ThinLTO、FatLTO 或无 LTO。
- codegen units。
- 链接上下文和内联决策。

推荐 ID：

```text
aarch64-rustls-common-state-pair-48-56
```

版本、编译器和构建参数属于验证元数据，不属于 detector identity。当暂时无法确定完整构建身份时，文档和 evidence 必须说明签名来自哪个已验证二进制，不得暗示覆盖某个库版本的所有构建。

## 13. 新增 Candidate 的规则

新增机器码 candidate 时必须：

1. 保留已有已验证 candidate，禁止直接替换旧签名。
2. 为新 candidate 使用新的稳定 ID。
3. 确认目标架构。
4. 确认每个 required endpoint 的真实语义。
5. 在真实 stripped binary 上验证每条签名唯一命中。
6. 验证完整 inbound/outbound closure。
7. 使用真实 agent 做端到端采集。
8. 回归至少一个旧 candidate 对应的真实 agent。
9. 记录来源版本、编译形态、offset、pattern 长度和验证结果。

禁止为了适配新版本而随意缩短旧 pattern。若引入 masked pattern、控制流检查或调用关系验证，必须作为新的 detector/evidence strategy，而不是静默改变旧 candidate 的含义。

## 14. Finder 与 Consumer 能力分离

Finder 能生成 plan，不代表任意 consumer 都能执行它。例如 finder 可以解析 Go pclntab，而某个 standalone collector 可能不支持 Go ABI capture。

因此必须分别表达：

```text
Detector capability:
  能否从目标生成完整且可信的 plan

Consumer capability:
  指定 collector/runtime 能否执行该 plan

Validation evidence:
  该组合是否在对应架构上经过真实 E2E
```

推荐使用复合能力键：

```rust
struct CapabilityKey {
    architecture: Architecture,
    provider: TlsProvider,
    source: ProbeSource,
    resolver: ResolverId,
    consumer: ProbeConsumer,
}
```

启动流程必须在运行目标前完成 consumer capability 校验。缺失能力必须 fail-fast，禁止在运行中静默降级成低覆盖模式。

## 15. 错误传播

检测发生在启动阶段，因此适用启动 fail-fast 原则：

- ELF 损坏：错误。
- offset 溢出：错误。
- candidate 歧义：错误。
- 已选择 detector 内部状态损坏：错误。
- provider 显式指定但没有完整 candidate：错误。
- consumer 不支持生成的 plan：错误。

以下情况可以局部继续：

- auto 模式下某个正常 candidate 返回 `NoMatch`。
- 某架构分支明确 `Inapplicable`。
- diagnostic 模式收集到一个 candidate 的普通不匹配证据。

禁止将任意 `DetectionError` 统一转换为 `NoMatch` 后继续 fallback，否则会把实现错误伪装成目标不受支持。

## 16. 面向对象与代码组织

相关状态和行为必须聚合到 detector struct：

```rust
struct RustlsStaticCatalog {
    candidates: Vec<Box<dyn ProbeDetector>>,
    selection: SelectionPolicy,
}
```

应该通过方法表达：

- applicability 判断。
- child 调度。
- evidence 聚合。
- closure 验证。
- ambiguity 判断。

纯字节搜索、checked integer conversion 等无状态工具可以作为私有工具函数，但应放在明确的 utils 模块，避免产生大量只增加跳转的伪 helper。

模块应优先按稳定职责拆分。完整目标路径以 `targeted-file-paths.zh.md` 为准，其顶层结构如下：

```text
probe_detector/
├─ mod.rs                 # 仅 re-export
├─ contract/              # contract 自身的层次化 namespace
│  ├─ detector/
│  ├─ detection/
│  ├─ candidate/
│  ├─ selection/
│  ├─ identity/
│  └─ capability/
└─ detector/
   └─ tls/
      ├─ rustls/
      ├─ boringssl/
      ├─ openssl/
      ├─ go_tls/
      ├─ gnutls/
      └─ nss/
```

具体目录应在实施时根据文件数量调整；不得为了匹配示意图创建只有一次转发的空层级。

## 17. `fast` 与 `detect`

`fast` 和 `detect` 必须共享 detector tree，禁止维护两套会逐渐漂移的 provider 识别逻辑。

差异应该由执行模式和 selection policy 表达：

```text
fast
  → FirstComplete
  → 只保留启动所需 evidence
  → 返回一个可执行 plan

detect
  → CollectAll
  → 在需要展开的各层显式配置 CollectAll
  → 保留该诊断模式承诺范围内的 evidence tree
  → 展示 matched/no-match/inapplicable/ambiguous
```

两种模式对单个 candidate 的成立条件必须完全一致。

## 18. 可观测性

检测报告至少应包含：

```text
detector_path
architecture
provider
source
resolver
candidate_id
outcome
required_endpoints
matched_endpoints
probe_points
consumer_support
validation_status
```

建议输出层次化 evidence tree，使用户能够看出：

```text
Rustls
└─ static
   └─ aarch64
      ├─ common-state-pair-52-64: no-match
      └─ common-state-pair-48-56: matched
         ├─ buffer: unique @ 0x...
         └─ take: unique @ 0x...
```

## 19. 测试与验证

本模块的功能完成声明必须依赖真实端到端测试，不能只依赖 parser mock 或单元测试。

最低验证集合：

1. 新 candidate 对应的真实 agent 能产生 outbound request。
2. 能收到 inbound response。
3. target 正常退出。
4. candidate evidence 指向预期 detector path。
5. 至少一个旧 candidate 的真实回归仍成功。
6. 不匹配的其他架构 candidate 没有被执行或误选。
7. consumer capability 不满足时在目标启动前失败。

不同架构的验证结果必须分别记录。x86_64 成功不能作为 aarch64 成功的证据，反之亦然。

## 20. 评审检查表

新增或修改 detector 时，评审者应检查：

- 是否实现统一 `ProbeDetector` 接口。
- 分支和叶子是否可由调用者统一处理。
- 是否错误地固定了树的层数。
- 是否使用目标 ELF 的真实架构。
- 是否把不同架构的机器码混入同一 candidate。
- 是否明确区分 `Inapplicable`、`NoMatch`、`Ambiguous` 和 error。
- 是否声明聚合策略。
- 是否要求完整 probe closure。
- 是否保留完整 detector path 和 evidence。
- 是否把 finder capability 与 consumer capability 分开。
- 是否追加而不是破坏旧 candidate。
- 是否完成新旧真实 agent E2E。
- 是否引入了只增加跳转层级的伪抽象。

## 21. 规范性总结

Probe Detector 的基本单元不是 provider、architecture 或 pattern，而是统一的递归检测对象：

```text
ProbeDetector
  = leaf detector
  | composite detector containing ProbeDetector children
```

框架必须允许：

```text
Provider
  → Architecture
    → Version
      → Compiler
        → Codegen shape
          → Evidence strategy
```

也必须允许 provider 使用完全不同的树：

```text
OpenSSL
  → Shared library discovery
    → Candidate path
      → Exported symbol closure
```

无论树如何分叉，调用接口、结果语义、错误语义、证据路径和 consumer capability 校验必须保持一致。
