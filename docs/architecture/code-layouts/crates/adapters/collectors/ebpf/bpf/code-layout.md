状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/adapters/collectors/ebpf/bpf/ # 组合观测程序并共享 maps，同一配置在 daemon 启动时加载一次
├── [x] live_observation.bpf.c # 组合各领域程序入口，生成长期 runtime 使用的观测对象
├── [x] units/ # 按职责组织程序入口并复用领域 helpers，组合对象共享同一组 maps
│   ├── [x] definitions.h # 汇总组合对象使用的领域定义
│   ├── [x] process/ # 共享进程维护与可选进程观测分别组织
│   │   ├── [x] context.bpf.c # 进程身份、launch 绑定及 fork/exec/exit 维护；提供依赖节点所需的共享上下文
│   │   ├── [x] observation.bpf.c # 进程 syscall 观测，依赖 context
│   │   └── [x] mapping.bpf.c # TLS 覆盖所需的映射事件，依赖 context
│   ├── [x] fd/ # 文件与网络共用的描述符程序
│   │   ├── [x] lifecycle.bpf.c # close/dup/fcntl 等配对程序及 FD 状态，依赖 process/context
│   │   └── [x] io.bpf.c # 公共 read/write/writev 配对程序，依赖 lifecycle；stdio、socket 与网络处理由共享配置 maps 控制
│   ├── [x] file/ # 文件路径采集程序源文件
│   │   ├── [x] open.bpf.c # 文件打开及 FD 关联，依赖进程与 FD 节点
│   │   ├── [x] mutation.bpf.c # 文件系统修改操作，按实际身份和路径资源声明依赖
│   │   └── [x] io.bpf.c # 文件 I/O 汇总的 vfs 和 security_file 程序，读取进程身份及所需 maps
│   ├── [x] network/ # 网络操作专属程序
│   │   └── [x] transport.bpf.c # socket/connect/accept/send/recv 程序，复用进程、FD 和公共 I/O 状态
│   └── [x] tls/ # 按后端支持条件分别组织 TLS 程序
│       ├── [x] ssl.bpf.c # OpenSSL 与 BoringSSL 入口及共用返回程序，各配对及共享状态由同一节点持有
│       ├── [x] rustls.bpf.c # rustls 程序及连接状态
│       ├── [x] gnutls.bpf.c # GnuTLS 程序及配对状态
│       ├── [x] nss.bpf.c # NSS/NSPR 程序及配对状态
│       └── [x] go.bpf.c # Go TLS 程序及配对状态
├── [x] common/ # 内核类型、寄存器读取和共享常量
│   ├── [x] constants.h # 定义采集容量、长度及内核常量
│   ├── [x] helpers.h # 封装共享内核辅助操作
│   ├── [x] kernel_types.h # 声明采集所需内核类型
│   ├── [x] map_flags.h # 定义 BPF map 配置标记
│   └── [x] uprobe_registers.h # 按架构读取用户态探针参数
├── [x] abi/ # 内核事件与用户态解码器共享的字节布局
│   ├── [x] observation.h # 定义公共事件头、种类和端点
│   │   └── [x] struct actrail_event_header # 携带事件长度、种类、时间和进程身份字段
│   ├── [x] process.h # fork、exec、exit、signal 及映射事件
│   │   ├── [x] struct actrail_process_fork_event # 携带 fork 的父子进程信息
│   │   └── [x] struct actrail_exec_file_identity # 表达实际执行文件的身份
│   ├── [x] network.h # 网络操作事件布局
│   │   └── [x] struct actrail_network_event # 保存网络操作、端点与返回结果
│   ├── [x] fd_io.h # 文件描述符 I/O 与 socket 释放事件
│   │   └── [x] struct actrail_fd_io_event # 保存描述符操作及 I/O 计数
│   ├── [x] file_path.h # 文件操作路径、阶段和参数布局
│   │   └── [x] struct actrail_file_event # 携带路径角色、操作阶段与 syscall 结果
│   └── [x] payload.h # TLS、socket 与 stdio 的载荷和完成事件
│       ├── [x] struct actrail_socket_payload_event # 表达 socket 字节采集段
│       ├── [x] struct actrail_stdio_payload_event # 表达 stdio 字节采集段
│       └── [x] struct actrail_tls_direct_capture_event # 表达由 BPF 复制得到的 TLS 明文段
├── [x] runtime/ # 各领域共享的事件传输、身份和 trace 查询
│   ├── [x] event_transport.h # 定义组合对象的共享输出 map、暂存空间及提交协议，随 runtime 保留
│   │   └── [x] struct actrail_event_scratch # 保存构造事件所需的有界暂存空间
│   ├── [x] process_identity.h # 定义共享的进程身份 maps 及布局，解析观测者 namespace 坐标并记录失败计数
│   │   └── [x] struct actrail_observer_pid_namespace # 标识用户态观测者所在的 PID namespace
│   ├── [x] trace_membership.h # 定义共享 trace 归属与观察深度，各程序读取同一 map 中的进程绑定
│   │   ├── [x] struct actrail_fork_trace_binding # 保存 fork 继承所需的 trace 关联
│   │   └── [x] struct actrail_process_observation_scope # 保存当前进程的观测范围
│   ├── [x] process_generation.h # 定义共享进程代际与线程身份状态，程序从同一内核实例读取
│   │   └── [x] struct actrail_process_identity # 关联内核进程与启动代际
│   ├── [x] endpoint.h # 提取和规范化网络端点
│   ├── [x] cas_compat.h # 封装内核支持范围内的比较交换操作
│   └── [x] fetch_add_compat.h # 封装计数递增的实现差异
├── [x] launch_binding/ # 将暂停 launch 目标的登记信息应用于首次 exec
│   ├── [x] binding.h # 定义登记匹配与消费；观测和 LSM 复用同一组 pending_exec maps
│   │   ├── [x] struct actrail_pending_exec_binding # 保存待执行进程的 trace 及采集绑定
│   │   └── [x] struct actrail_launch_binding_key # 用进程坐标和代际定位待执行绑定
│   └── [x] impl/ # 按内核能力提供 launch 绑定存储
│       ├── [x] task_storage.h # 使用 task storage 将绑定附着到实际任务
│       └── [x] pid_generation_hash.h # 使用 PID 与进程代际的哈希索引保存绑定
├── [x] process/ # 进程事件、执行参数、身份解析和生命周期状态
│   ├── [x] state.h # 保存 fork 关联与待完成 exit 操作
│   │   └── [x] struct actrail_observer_fork_binding # 保存观测者坐标下的父子关联
│   ├── [x] observe.h # 保存 exec/fork 参数和执行中操作
│   │   ├── [x] struct actrail_pending_process_exec # 连接 exec 入口采样与返回事件
│   │   └── [x] struct actrail_pending_process_fork # 连接 fork 入口采样与返回事件
│   ├── [x] identity_resolution.h # 缓存内核任务到观测者进程身份的解析结果
│   │   └── [x] struct actrail_process_identity_resolution # 保存当前解析到的身份坐标
│   ├── [x] file_identity.h # 从执行文件读取稳定文件身份
│   ├── [x] mapping.h # 定义文件映射观测标志，程序入口归属 units/process/mapping
│   ├── [x] programs.h # 汇总进程观测与生命周期所需 helpers，程序入口归属 units 中的物理节点
│   └── [x] observation/ # exec 与 fork 观测的共享实现
│       ├── [x] exec_emit.h # 构造执行尝试与参数事件
│       ├── [x] argv.h # 按内核循环能力采集 argv，支持 tail-call continuation
│       ├── [x] exec.h # 保存执行操作并报告失败
│       └── [x] fork.h # 保存 fork 操作并报告结果
├── [x] fd/ # 文件描述符、底层对象、继承和清理状态
│   ├── [x] types.h # 定义描述符分类、对象、索引和待完成操作
│   │   ├── [x] struct actrail_fd_state # 保存单个描述符关联的对象与标记
│   │   └── [x] struct actrail_fd_object_state # 保存多个描述符共享的底层对象状态
│   ├── [x] maps.h # 定义组合对象内共享 FD/对象状态与操作 pending maps
│   ├── [x] descriptor.h # 查找描述符并取得对象快照
│   ├── [x] index.h # 分配和释放进程描述符索引槽位
│   ├── [x] lifecycle.h # 处理描述符创建、复制、关闭及对象引用
│   ├── [x] sweep.h # 在 fork、exec、exit 和 close_range 时有界清理描述符
│   ├── [x] suppressed.h # 保存和继承需要排除采集的描述符
│   │   └── [x] struct actrail_suppressed_fd_key # 用进程身份与描述符定位排除项
│   └── [x] programs.h # 汇总 FD 生命周期 helpers，程序入口归属 units/fd/lifecycle
├── [x] network/ # 网络 syscall 的入口、返回和地址关联
│   ├── [x] state.h # 保存尚未返回的网络操作
│   │   └── [x] struct actrail_pending_net_op # 保留网络操作参数供返回阶段使用
│   ├── [x] observe.h # 读取网络参数并构造观测事件
│   └── [x] programs.h # 汇总网络与 socket 采集 helpers，程序入口归属 transport 与公共 I/O 节点
├── [x] file/ # 文件路径、打开、修改与 I/O 完成观测
│   ├── [x] state.h # 保存采集配置和待完成文件操作
│   │   ├── [x] struct actrail_file_config # 配置文件领域的采集行为
│   │   └── [x] struct actrail_pending_file_completion # 连接文件操作入口与结果
│   ├── [x] paths.h # 从文件操作参数读取和输出路径
│   ├── [x] open.h # 捕获文件打开及其描述符关联
│   ├── [x] mutation.h # 汇总文件系统修改操作的观测依赖，程序入口归属 units/file/mutation
│   ├── [x] completion.h # 输出文件操作完成结果
│   ├── [x] observe.h # 组织文件操作的公共观测行为
│   ├── [x] programs.h # 汇总文件操作 helpers，程序入口归属 open、mutation 与 FD 节点
│   └── [x] io/ # 以底层文件对象聚合读写指标
│       ├── [x] state.h # 保存 I/O 汇总和待完成操作状态
│       ├── [x] objects.h # 识别文件对象并维护聚合对象关联
│       └── [x] programs.h # 实现文件 I/O 汇总计算，由公共 I/O 程序调用
├── [x] payload/ # socket 和 stdio 明文采集及完成标记
│   ├── [x] socket_types.h # socket 采集配置、描述符身份和待完成操作
│   │   ├── [x] struct actrail_socket_payload_fd_state # 保存 socket 描述符的采集状态
│   │   └── [x] struct actrail_pending_socket_payload_op # 保存用户态缓冲与操作完成关联
│   ├── [x] socket_state.h # 拥有 socket 字节序列和待完成采集 map
│   ├── [x] socket_tls.h # 识别 socket 与 TLS 流的采集关系
│   ├── [x] socket_capture.h # 按配置预算复制 socket 缓冲字节
│   ├── [x] socket_emit.h # 输出 socket 载荷和完成事件
│   ├── [x] stdio_capture.h # 复制标准输入输出字节并保留流方向
│   └── [x] programs.h # 汇总共享读写 helpers，程序入口归属 fd/io；stdio 与 socket 通过配置 maps 控制处理
└── [x] tls/ # TLS 函数调用的缓冲、返回值与明文采集状态
    ├── [x] state.h # 拥有 TLS 配置、未完成操作和库专属连接状态
    │   ├── [x] struct actrail_tls_payload_config # 选择 TLS 明文采集后端及预算
    │   └── [x] struct actrail_pending_tls_payload_op # 保存调用缓冲和完成关联
    ├── [x] capture.h # 从 TLS 调用缓冲中读取明文
    ├── [x] completion.h # 输出 TLS 操作完成与实际长度
    ├── [x] diagnostics.h # 记录 TLS 覆盖、读取和容量诊断
    ├── [x] rustls.h # 处理 rustls 连接状态和读写布局
    ├── [x] boringssl.h # 处理 BoringSSL 调用和明文缓冲
    └── [x] programs.h # 汇总 TLS 采集 helpers，entry/return 程序归属对应的物理节点
```
