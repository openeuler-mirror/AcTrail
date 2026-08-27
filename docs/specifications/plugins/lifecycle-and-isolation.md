# 插件生命周期与隔离规范

> 本文规定维护者实现或审查插件发现、加载、授权、运行期隔离和卸载时必须保持的行为。

状态：已实现
范围：插件发现、加载、授权、运行期失败和卸载

## 生命周期

1. 安装和目录发现不得加载插件、创建 exporter、打开业务输出文件或发起网络请求。
2. 目录发现只能产生候选视图；运行实例必须由管理员显式加载、启动清单或持久化注册创建。
3. 加载必须先完成 manifest、配置 schema、capability/grant、artifact 和资源限制校验。
   启动阶段的错误必须明确失败，不得用其他路径、exporter 或默认权限继续。
4. 插件配置仅在实例加载时读取。修改磁盘文件不得隐式改变活动实例。
5. 删除候选包不得隐式卸载实例；卸载必须是显式管理操作。

## 授权

1. manifest 声明 capability 只是权限请求，不构成授权。
2. 管理员 grant 必须与声明匹配；缺少必需 grant 或授予未声明 capability 都必须拒绝加载。
3. 参数化 grant 必须限制到具体环境变量、规则类型、路径或 endpoint 范围；插件在运行期
   仍不得超出该范围。
4. builtin 运行时不得绕过统一插件生命周期和授权校验。
5. 控制 socket 必须保持 host-root peer 边界；Web 管理面不得把任意文件路径直接转交 daemon。

## 故障隔离

1. observation consumer 的慢速 I/O 和网络交付不得阻塞 recording 热路径。
2. 单个 observation consumer 队列满、trap、超时或下游失败，只能影响该实例；不得使
   daemon、trace recording 或其他插件异常失败。
3. control-decider 的失败必须按所属控制面的显式 failure decision 处理，不得从插件错误
   推导未配置的隐式放行或拒绝。
4. LLM codec 的 trap、fuel 耗尽、非法输出或 `no_match` 不得删除原始 payload；调用方按
   [LLM Codec ABI](../../reference/plugin-api/llm-codec.md) 的链式回退继续处理。
5. 所有运行时资源限制、timeout、队列容量、读取上限和重试次数必须来自 manifest 或配置，
   禁止无限队列、无限等待或无限重试。

## 持久化所有权

启动清单与运行时持久化注册是两个独立所有权模型。同一实例不得同时由两者管理。启动
清单在 daemon 启动时应用其 failure policy；运行时 `--persist` 记录在后续启动时恢复。
