# LSM Grey 命令执行授权

本文说明基于内核执行参数副本的 grey 授权设计。Agent 启动的子进程尝试执行命令时，LSM 先查询内核策略。静态规则直接返回 allow 或 deny。grey 规则通过 daemon 调用访问控制 WASM 插件，由插件请求外部安全服务并解释响应。

设计的关键是读取 `linux_binprm` 对应的执行参数区域。内核已经为本次 exec 复制参数，模块从这份副本构造授权请求，避免重新读取原始用户态 argv 导致参数不一致。

## 参与者与职责

| 参与者 | 职责 |
| --- | --- |
| Agent | 启动执行工具或命令的子进程 |
| 子进程 | 发起 `execve` 或 `execveat`，例如执行带 GET 或 POST 参数的 curl |
| exec 内核路径 | 复制执行参数，调用 LSM，决定是否继续执行目标程序 |
| 内核参数区域 | 保存本次执行使用的参数，由 `bprm->mm`、`bprm->p` 和 `bprm->argc` 定位 |
| BPF LSM | 检查已有 LSM 裁决、受控范围和策略，选择直接裁决或 grey 分支 |
| 内核 grey 模块 | 读取执行参数副本，维护有界等待队列，接收裁决并唤醒原线程 |
| `LsmGreyBroker` / `GreyContext` | 读取设备请求，构造命令上下文，携带剩余期限分发并写回裁决 |
| `ControlPluginRuntime` | 定位规则指定的插件实例并调用控制插件 |
| Actrail WASM 底座 | 执行组件，提供上下文查询和受限的通用 HTTP 接口 |
| 访问控制 WASM 插件 | 构造业务请求，解释服务响应，生成 allow 或 deny |
| 外部安全服务 | 根据参数、上下文及自身规则计算分析结果 |

## 完整时序

下图以原生 ELF 程序的执行为范围。所有参数读取都发生在 grey 分支中。`exec kfunc` 表示接收当前 `bprm` 的模块入口。

```mermaid
sequenceDiagram
    autonumber
    actor Agent
    participant Child as Agent 子进程
    participant Exec as 危险调用<br/>execve / execveat
    participant Args as 内核执行参数区域<br/>bprm 的新 mm
    participant LSM as BPF LSM<br/>bprm_check_security
    participant Policy as 内核策略 map
    participant Grey as 内核 grey 模块<br/>参数采集与等待队列
    participant Broker as actraild<br/>LsmGreyBroker / GreyContext
    participant Runtime as actraild<br/>ControlPluginRuntime
    participant Wasm as Actrail WASM 底座<br/>Wasmtime / host 接口
    participant Plugin as 访问控制 WASM 插件
    participant Service as 外部安全服务

    Agent->>Child: 启动子进程
    Note over Agent,Child: 子进程继承受控 trace 范围
    Child->>Exec: 尝试执行命令及参数
    Exec->>Args: 将 argv 复制到本次执行的参数区域
    Exec->>LSM: 进入 sleepable hook，传入 bprm
    LSM->>LSM: 判断已有 LSM 裁决与受控范围

    alt 已有 LSM 拒绝
        LSM-->>Exec: 保留已有错误
    else 不属于受控范围
        LSM-->>Exec: 本后端返回 0
    else 属于受控范围
        LSM->>Policy: 按可执行文件资源匹配命令规则
        Policy-->>LSM: 裁决类型、插件目标、期限及 fallback
        alt allow
            LSM-->>Exec: 返回 0
        else deny
            LSM-->>Exec: 返回 -EACCES
        else grey 动态等待未启用
            LSM-->>Exec: 按规则 fallback 返回
        else grey 动态等待已启用
            LSM->>Grey: exec kfunc(bprm, 规则及访问上下文)
            Grey->>Args: 按 argc 读取参数页，处理跨页字符串
            Args-->>Grey: 本次执行参数及完整性状态
            Grey->>Grey: 释放采集时的页引用和 mmap 读锁
            alt 完整参数授权所需的信息不完整或读取失败
                Grey-->>LSM: 拒绝本次命令执行
                LSM-->>Exec: 返回 -EACCES
            else 参数完整
                Grey->>Grey: 检查设备连接、容量和并发上限
                alt 无法接纳请求
                    Grey-->>LSM: 返回通道错误
                    LSM-->>Exec: 按规则 fallback 返回
                else 请求入队
                    Grey->>Grey: 保存请求快照、设置截止时间并等待
                    Note over Child,Grey: 当前 exec 线程阻塞，目标程序尚未运行
                    Note over Args,Grey: 原始用户态 argv 的后续改写<br/>不作为本次授权参数来源

                    Broker->>Grey: 从设备读取请求
                    Grey-->>Broker: 请求 ID、参数、访问上下文及截止时间
                    Broker->>Broker: 构造 command_execution_context
                    Broker->>Runtime: 调用指定插件，传递剩余期限
                    Runtime->>Wasm: 执行控制组件
                    Wasm->>Plugin: decide(访问上下文)
                    Plugin->>Wasm: 查询 command-execution.v1 上下文
                    Wasm-->>Plugin: 可执行文件、argv 及完整性信息
                    Plugin->>Plugin: 构造安全分析服务的业务请求
                    Plugin->>Wasm: http-host.request(method, URL, headers, body)
                    Wasm->>Wasm: 检查权限、允许的 URL、期限与大小限制
                    Wasm->>Service: 发送 HTTP 请求
                    Service->>Service: 根据上下文与自身规则分析
                    Service-->>Wasm: HTTP 响应
                    Wasm-->>Plugin: 状态码、响应头、正文或传输错误
                    Plugin->>Plugin: 解释响应并生成裁决
                    Plugin-->>Wasm: allow / deny 或执行错误
                    Wasm-->>Runtime: 插件结果
                    Runtime-->>Broker: 裁决或执行错误
                    Broker->>Broker: 应用插件失效与执行错误策略
                    Broker->>Grey: 写回请求 ID 与裁决
                    Grey->>Grey: 校验请求仍有效，唤醒原线程
                    Grey-->>LSM: 返回裁决或等待错误
                    LSM->>LSM: 确定最终裁决，按配置输出审计
                    LSM-->>Exec: 返回 0 或 -EACCES
                end
            end
        end
    end

    alt 本次执行通过相关检查
        Exec->>Exec: 继续加载目标程序
        Note over Child,Exec: exec 成功后，子进程运行目标程序映像<br/>目标程序使用内核准备的参数
    else 本后端拒绝执行
        Exec-->>Child: exec 返回权限错误
    end
```

LSM 返回 0 表示本后端允许继续执行。exec 后续仍可能因其他检查或程序加载错误而失败。

## 参数一致性与异常处理

内核执行参数副本是本次 argv 授权的数据来源。模块读取 `bprm->mm` 中从 `bprm->p` 开始的参数字符串，并按 `bprm->argc` 确定结束位置。授权请求只保存采集结果，等待期间不保留参数页映射和采集用的内存映射读锁。exec 路径自身仍可能持有其他内核锁。

WIT 的 argv 使用 UTF-8 字符串。当前 daemon 对非 UTF-8 字节使用替换字符解码，因此插件侧的逐字节一致性范围是有效 UTF-8 参数。内核参数来源的一致性不改变这项文本接口限制。

完整参数授权要求采集结果覆盖全部 argv。若达到采集上限或读取失败，拒绝这类命令授权，避免将未知参数视为已授权。参数完整性错误与普通通道错误分别处理，完整性错误不能转成允许型 fallback。

| 条件 | 处理 |
| --- | --- |
| 静态 allow / deny | 内核直接裁决，不采集 argv，不请求安全服务 |
| 动态 grey 未启用 | 使用规则配置的 fallback |
| 参数读取失败或超限 | 对依赖完整 argv 的授权拒绝执行 |
| 设备未连接、队列满或达到并发上限 | 使用规则配置的 fallback |
| 指定插件不存在或执行期间被卸载 | 拒绝访问 |
| 插件执行失败，包括未处理的 HTTP 错误 | 使用规则配置的 fallback |
| 内核等待超时、被中断或 daemon 断开 | 结束等待，使用规则配置的 fallback |
| 裁决到达时请求已结束 | 拒绝迟到的回复 |

主图展示请求正常返回的路径。内核等待期限独立于外部服务运行，用户态处理停滞时仍会结束等待。需要确保外部授权成功才允许执行的规则，应选择 deny fallback，并启用动态 grey。

## WASM 与安全服务边界

`http-host.request` 提供通用 HTTP 传输。插件选择已授权 URL、方法、请求头和正文，宿主执行权限、地址、时限及正文大小限制。安全服务的 JSON 字段及响应到 allow/deny 的映射属于插件逻辑。接口定义见[控制插件 API](../../reference/plugin-api/control-decider.md#http-请求)。

例如，插件可以将 curl 的参数交给服务分析，允许 `-X GET`，拒绝 `-X POST`。拒绝发生在 curl 的 exec 阶段，目标 curl 程序尚未开始发送 HTTP 请求。daemon 和安全服务独立运行在受控 Agent 的进程树之外，服务使用自己的规则库和模板文件。

