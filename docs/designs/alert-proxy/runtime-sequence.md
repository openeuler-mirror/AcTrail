# 告警代理运行时序

## 1. actraild 启动

### 1.1 初始化配置

**1.1.1** `actraild` 解析完整 OperatorConfig，并校验独立的 `[alert_forwarding]` 子配置。

**1.1.2** daemon 注册 builtin `alert-forwarding` plugin，并从独立 plugin config file加载 `enabled`、`all_categories` 和 `categories`。

**1.1.3** daemon 创建连接状态和不可变类别过滤快照；建立有效 proxy link 时创建该 generation 的有界 forwarding queue。

### 1.2 建立初始 proxy 连接

**1.2.1** builtin 配置的 `enabled=false` 时，effective enabled 保持 `false`，daemon 不要求 proxy 存在。

**1.2.2** builtin 配置的 `enabled=true` 时，supervisor 先尝试连接配置的 daemon ingress UDS。

**1.2.3** 连接成功时复用现有 proxy，并把进程所有权标记为 borrowed。

**1.2.4** socket 不存在或拒绝连接时，supervisor 使用配置的 proxy executable 和 proxy config path 拉起 `actraild-alert-proxy`。

**1.2.5** supervisor 在配置的 startup timeout 内轮询 ingress ready，并回收提前退出的 child。

**1.2.6** daemon 发送 `ProducerHello`，收到合法 `ProducerWelcome` 后创建 link writer owner。

**1.2.7** writer ready 后，daemon 原子发布类别过滤快照并把 effective enabled 设置为 `true`。

**1.2.8** 配置、路径、进程创建、初始连接、握手或link owner创建失败使 daemon 启动失败。

## 2. actraild-alert-proxy 启动

### 2.1 初始化进程

**2.1.1** proxy 严格解析独立 TOML 配置。

**2.1.2** proxy 校验绝对 UDS path、subscriber bind address、token、容量、timeout、frame limit 和线程栈。

**2.1.3** proxy 创建 `SubscriberRegistry` 和 `AlertBroadcaster`。

### 2.2 开放 daemon ingress

**2.2.1** proxy 清理经过 socket 类型核验的 stale UDS path，然后 bind daemon ingress listener。

**2.2.2** proxy 设置 socket 权限，并创建 daemon ingress accept owner。

**2.2.3** daemon ingress accept owner等待全局 ready gate，不在subscriber listener ready前返回 `ProducerWelcome`。

**2.2.4** accept owner限制待握手连接数量；worker在ready gate开放后返回welcome，并且只允许一个完成握手的活动producer。

### 2.3 开放 subscriber listener

**2.3.1** proxy bind 配置的 TCP address，设置 nonblocking，并创建 subscriber accept owner。

**2.3.2** proxy 在配置的 subscriber 连接上限内，为每个 accepted socket 创建独立 `SubscriberSession`。

**2.3.3** 两个 listener 和两个 accept owner 全部 ready 后，proxy开放全局 ready gate，输出 ready并进入 signal wait。

## 3. 外部 subscriber 接入

### 3.1 建立 session

**3.1.1** subscriber 建立 TCP 连接并发送长度前缀 handshake request。

**3.1.2** session 校验 frame、协议版本、client ID 和 opaque bearer token。

**3.1.3** 校验成功后，session 分配 session ID，返回 heartbeat interval，并在进入请求循环前注册到 `SubscriberRegistry`。

### 3.2 提交订阅

**3.2.1** subscriber 发送带 request ID 的 subscribe request。

**3.2.2** session 校验 topics的非空值、长度、数量和允许字符，校验severity和空tags object。

**3.2.3** session先把相同request ID、`accepted`和实际topics写入自己的outbound queue。

**3.2.4** 确认帧成功入队后，session才原子替换订阅快照。

**3.2.5** 同一outbound queue保持确认帧先于该订阅产生的第一条告警推送。

**3.2.6** session reader持续处理后续subscribe和pong；session writer持续处理独立outbound queue，并按heartbeat interval发送带nonce的ping。

## 4. 告警产生与转发

### 4.1 actraild 接纳告警

**4.1.1** producer 或 daemon enforcement 把告警提交到 `AlertIngress`。

**4.1.2** `AlertIngress` 完成 token、definition、payload 和去重校验，并向主 Storage 提交。

**4.1.3** Storage 返回 `Stored` 时，`AlertIngress` 使用内存 definition metadata 和已校验 draft 构造标准告警。

### 4.2 actraild 接纳 Sandbox 资源告警

**4.2.1** `sandbox-resource-alert` 从 Hand observation 产生带真实 Sandbox source 的类型化告警。

**4.2.2** plugin consumer 对独立 Sandbox Alert DB 的有界 queue 执行 nonblocking `try_send`。

**4.2.3** database owner 使用独立 SQLite connection 提交结构化告警事务。

**4.2.4** 只有事务提交成功的告警才转换为标准 `ForwardAlert`。

**4.2.5** Sandbox 告警不进入 `AlertIngress`、主 Storage 或 trace 关联，不生成虚假 `trid`。

### 4.3 builtin plugin 过滤

**4.2.1** forwarding plugin 读取 effective enabled。

**4.2.2** disabled 时立即返回，不进入 forwarding queue。

**4.2.3** enabled 时读取不可变类别过滤快照。

**4.2.4** 类别不匹配时立即返回。

**4.2.5** 类别匹配时对有界 queue 执行 `try_send`。

**4.2.6** queue 满或关闭时只丢弃当前外发副本。

### 4.4 proxy 广播

**4.3.1** daemon link writer把标准告警编码成内部二进制 frame并写入持久 UDS。

**4.3.2** proxy ingress解码不含message ID的 `ForwardAlert`，并对有界 broadcast queue 执行 `try_send`；queue 满时只丢弃当前外发副本。

**4.3.3** broadcaster线程取得告警后，在 registry lock 内取得已握手 session 快照，然后释放 lock。

**4.3.4** broadcaster对每个session只取得一次delivery lock，按该时刻的订阅快照执行topic和severity匹配，形成匹配session列表。

**4.3.5** 匹配列表非空时，broadcaster为该告警生成一次UUID，只编码一次长度前缀JSON frame，并以 `Arc<[u8]>` 共享给匹配session。

**4.3.6** broadcaster对每个匹配session queue执行 `try_send`。

**4.3.7** 某个session queue满时只关闭该session，不阻塞其他session。

## 5. Heartbeat 与断连

### 5.1 daemon link

**5.1.1** daemon link writer按producer heartbeat interval发送带nonce的内部Heartbeat，不因告警流量停止探活；同一时刻只保留一个outstanding nonce。

**5.1.2** proxy返回相同nonce的HeartbeatAck，daemon link reader持续监测ack、EOF和HUP。

**5.1.3** write失败、EOF、HUP、错误ack或ack timeout时，link把失败generation提交给state owner。

**5.1.4** 只有失败generation仍是active generation时，state owner才把requested和effective enabled设置为 `false`，原子更新plugin config file并关闭当前UDS。

**5.1.5** link丢弃旧connection generation的queued items。

**5.1.6** 后续告警继续正常持久化，但不再进入forwarding queue。

### 5.2 subscriber session

**5.2.1** session writer按heartbeat interval发送ping，并保持最多一个outstanding nonce。

**5.2.2** reader收到匹配nonce的pong时清除outstanding ping。

**5.2.3** 其他合法inbound frame可以刷新peer activity，但不能清除outstanding ping或延长pong deadline。

**5.2.4** pong deadline、peer idle timeout、EOF、协议错误或I/O错误只关闭当前 session并从 registry移除。

## 6. Web 重新启用

### 6.1 配置提交

**6.1.1** Web 使用现有 plugin config API提交 `enabled=true` 和类别选择。

**6.1.2** daemon 先校验配置，不在 validate阶段拉起进程或建立连接。

**6.1.3** update阶段执行 proxy探测、必要拉起、连接和握手。

**6.1.4** 全部成功后才原子写入plugin config file、提交配置快照并设置effective enabled。

**6.1.5** 失败时 API返回错误，effective enabled保持 `false`。

## 7. 停止

### 7.1 停止 actraild

**7.1.1** daemon先排空已经接纳的 AlertIngress 写入，再把 effective enabled设置为 `false`，关闭 forwarding queue并停止 link writer。

**7.1.2** daemon回收自己拉起且已经退出的 proxy child状态。

**7.1.3** daemon不主动终止borrowed或owned proxy；proxy作为独立进程保持自己的生命周期。

### 7.2 停止 proxy

**7.2.1** proxy停止两个 accept owner，不再接受新 producer或subscriber。

**7.2.2** proxy关闭 producer connection和所有 session queue。

**7.2.3** session workers退出后，proxy删除自己创建的 daemon ingress socket path。
