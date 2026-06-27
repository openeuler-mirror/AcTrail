(module
  (import "actrail_host" "payload_read"
    (func $payload_read (param i32 i32 i64 i32 i32) (result i64)))

  (memory (export "memory") 1)
  (global $next (mut i32) (i32.const 4096))

  (data (i32.const 1024) "payload-1")

  (func (export "actrail_alloc") (param $len i32) (result i32)
    (local $ptr i32)
    (local.set $ptr (global.get $next))
    (global.set $next (i32.add (global.get $next) (i32.add (local.get $len) (i32.const 8))))
    (local.get $ptr)
  )

  (func (export "actrail_plugin_init") (param $ptr i32) (param $len i32) (result i32)
    (i32.const 0)
  )

  (func (export "actrail_observation_consume") (param $ptr i32) (param $len i32) (result i64)
    (if (i64.ne
          (call $payload_read
            (i32.const 1024)
            (i32.const 9)
            (i64.const 0)
            (i32.const 2048)
            (i32.const 16)
          )
          (i64.const -1))
      (then (return (i64.const -1)))
    )
    (i64.const 1)
  )
)
