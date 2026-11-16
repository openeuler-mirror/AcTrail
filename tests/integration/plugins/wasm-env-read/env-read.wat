(module
  (import "actrail_host" "env_read"
    (func $env_read (param i32 i32 i32 i32) (result i64)))

  (memory (export "memory") 1)
  (global $next (mut i32) (i32.const 4096))

  (data (i32.const 1024) "ACTRAIL_PLUGIN_ENV_SECRET")
  (data (i32.const 2048) "allowed-secret")

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
    (local $read i64)
    (local.set $read
      (call $env_read
        (i32.const 1024)
        (i32.const 25)
        (i32.const 3072)
        (i32.const 64)
      )
    )
    (if (i64.ne (local.get $read) (i64.const 14))
      (then (return (i64.const -1)))
    )
    (if (i32.ne
          (i32.load8_u (i32.const 3072))
          (i32.load8_u (i32.const 2048)))
      (then (return (i64.const -2)))
    )
    (if (i32.ne
          (i32.load8_u (i32.const 3085))
          (i32.load8_u (i32.const 2061)))
      (then (return (i64.const -3)))
    )
    (i64.const 1)
  )
)
