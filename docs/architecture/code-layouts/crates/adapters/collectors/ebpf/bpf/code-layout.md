状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/adapters/collectors/ebpf/bpf/ # 内核采集程序及其领域状态；由 live_observation.bpf.c 组合为观测对象
├── [x] live_observation.bpf.c # 组合公共定义、ABI、运行时和各领域程序
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
│   ├── [x] event_transport.h # 拥有事件暂存和输出 map，统一 ring/perf 提交与失败计数
│   │   └── [x] struct actrail_event_scratch # 保存构造事件所需的有界暂存空间
│   ├── [x] process_identity.h # 解析观测者命名空间坐标并记录失败计数
│   │   └── [x] struct actrail_observer_pid_namespace # 标识用户态观测者所在的 PID namespace
│   ├── [x] trace_membership.h # 查询进程的 trace 归属与观察深度
│   │   ├── [x] struct actrail_fork_trace_binding # 保存 fork 继承所需的 trace 关联
│   │   └── [x] struct actrail_process_observation_scope # 保存当前进程的观测范围
│   ├── [x] process_generation.h # 保存进程代际与 trace 内线程身份
│   │   └── [x] struct actrail_process_identity # 关联内核进程与启动代际
│   ├── [x] endpoint.h # 提取和规范化网络端点
│   ├── [x] cas_compat.h # 封装内核支持范围内的比较交换操作
│   └── [x] fetch_add_compat.h # 封装计数递增的实现差异
├── [x] launch_binding/ # 将暂停 launch 目标的登记信息应用于首次 exec
│   ├── [x] binding.h # 定义登记匹配、消费、失败输出和受抑制描述符安装
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
│   ├── [x] mapping.h # 观测可能改变 TLS 探针覆盖的文件映射
│   ├── [x] lifecycle_programs.h # 处理 fork、exec 与 exit 生命周期挂载点
│   └── [x] programs.h # 挂载进程 syscall 并输出尝试、参数和结果事件
├── [x] fd/ # 文件描述符、底层对象、继承和清理状态
│   ├── [x] types.h # 定义描述符分类、对象、索引和待完成操作
│   │   ├── [x] struct actrail_fd_state # 保存单个描述符关联的对象与标记
│   │   └── [x] struct actrail_fd_object_state # 保存多个描述符共享的底层对象状态
│   ├── [x] maps.h # 拥有描述符、对象和待完成 syscall 的内核 map
│   ├── [x] descriptor.h # 查找描述符并取得对象快照
│   ├── [x] index.h # 分配和释放进程描述符索引槽位
│   ├── [x] lifecycle.h # 处理描述符创建、复制、关闭及对象引用
│   ├── [x] sweep.h # 在 fork、exec、exit 和 close_range 时有界清理描述符
│   ├── [x] suppressed.h # 保存和继承需要排除采集的描述符
│   │   └── [x] struct actrail_suppressed_fd_key # 用进程身份与描述符定位排除项
│   └── [x] programs.h # 挂载描述符相关 syscall 并维护生命周期
├── [x] network/ # 网络 syscall 的入口、返回和地址关联
│   ├── [x] state.h # 保存尚未返回的网络操作
│   │   └── [x] struct actrail_pending_net_op # 保留网络操作参数供返回阶段使用
│   ├── [x] observe.h # 读取网络参数并构造观测事件
│   └── [x] programs.h # 挂载网络 syscall 入口和返回点
├── [x] file/ # 文件路径、打开、修改与 I/O 完成观测
│   ├── [x] state.h # 保存采集配置和待完成文件操作
│   │   ├── [x] struct actrail_file_config # 配置文件领域的采集行为
│   │   └── [x] struct actrail_pending_file_completion # 连接文件操作入口与结果
│   ├── [x] paths.h # 从文件操作参数读取和输出路径
│   ├── [x] open.h # 捕获文件打开及其描述符关联
│   ├── [x] mutation.h # 捕获修改文件系统对象的操作
│   ├── [x] completion.h # 输出文件操作完成结果
│   ├── [x] observe.h # 组织文件操作的公共观测行为
│   ├── [x] programs.h # 挂载文件路径与修改操作入口
│   └── [x] io/ # 以底层文件对象聚合读写指标
│       ├── [x] state.h # 保存 I/O 汇总和待完成操作状态
│       ├── [x] objects.h # 识别文件对象并维护聚合对象关联
│       └── [x] programs.h # 挂载读写操作并累计完成计数
├── [x] payload/ # socket 和 stdio 明文采集及完成标记
│   ├── [x] socket_types.h # socket 采集配置、描述符身份和待完成操作
│   │   ├── [x] struct actrail_socket_payload_fd_state # 保存 socket 描述符的采集状态
│   │   └── [x] struct actrail_pending_socket_payload_op # 保存用户态缓冲与操作完成关联
│   ├── [x] socket_state.h # 拥有 socket 字节序列和待完成采集 map
│   ├── [x] socket_tls.h # 识别 socket 与 TLS 流的采集关系
│   ├── [x] socket_capture.h # 按配置预算复制 socket 缓冲字节
│   ├── [x] socket_emit.h # 输出 socket 载荷和完成事件
│   ├── [x] stdio_capture.h # 复制标准输入输出字节并保留流方向
│   └── [x] programs.h # 挂载 payload 采集 syscall 程序
└── [x] tls/ # TLS 函数调用的缓冲、返回值与明文采集状态
    ├── [x] state.h # 拥有 TLS 配置、未完成操作和库专属连接状态
    │   ├── [x] struct actrail_tls_payload_config # 选择 TLS 明文采集后端及预算
    │   └── [x] struct actrail_pending_tls_payload_op # 保存调用缓冲和完成关联
    ├── [x] capture.h # 从 TLS 调用缓冲中读取明文
    ├── [x] completion.h # 输出 TLS 操作完成与实际长度
    ├── [x] diagnostics.h # 记录 TLS 覆盖、读取和容量诊断
    ├── [x] rustls.h # 处理 rustls 连接状态和读写布局
    ├── [x] boringssl.h # 处理 BoringSSL 调用和明文缓冲
    └── [x] programs.h # 组合不同 TLS 库的用户态探针程序
```
