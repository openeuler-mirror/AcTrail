(module
  ;; 导出线性内存，让 AcTrail 可以把请求数据写进插件内存。
  (memory (export "memory") 1)

  ;; 下一个可用字节位置，供下面这个极简分配器使用。
  (global $next (mut i32) (i32.const 1024))

  ;; AcTrail 写入输入数据前会先调用这个函数。
  ;; 插件返回一个字节偏移，表示 AcTrail 可以从这里写入 `$len` 字节。
  (func (export "actrail_alloc") (param $len i32) (result i32)
    (local $ptr i32)

    ;; 把当前空闲位置分配给本次请求。
    (local.set $ptr (global.get $next))

    ;; 推进下一个空闲位置。这个示例只分配，不回收内存。
    (global.set $next (i32.add (global.get $next) (i32.add (local.get $len) (i32.const 8))))

    ;; 把本次分配到的位置返回给 AcTrail。
    (local.get $ptr)
  )

  ;; AcTrail 加载插件时调用这个初始化函数。
  ;; 这个示例没有启动状态，也不解析配置，所以直接返回 0 表示成功。
  (func (export "actrail_plugin_init") (param $ptr i32) (param $len i32) (result i32)
    (i32.const 0)
  )

  ;; 命令执行请求进入这个控制插件时，AcTrail 会调用这个决策函数。
  ;; 这个示例不读取请求内容，固定返回“拒绝一次”。
  (func (export "actrail_control_decide") (param $ptr i32) (param $len i32) (result i64)
    ;; -1 表示 deny-once，也就是只拒绝当前这一次请求。
    (i64.const -1)
  )
)
