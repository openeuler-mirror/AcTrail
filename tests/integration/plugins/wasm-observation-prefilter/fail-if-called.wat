(module
  (memory (export "memory") 1)
  (global $next (mut i32) (i32.const 1024))
  (global $config_len (mut i32) (i32.const 0))

  (func (export "actrail_alloc") (param $len i32) (result i32)
    (local $ptr i32)
    (local.set $ptr (global.get $next))
    (global.set $next (i32.add (global.get $next) (i32.add (local.get $len) (i32.const 8))))
    (local.get $ptr)
  )

  (func (export "actrail_plugin_init") (param $ptr i32) (param $len i32) (result i32)
    (global.set $config_len (local.get $len))
    (i32.const 0)
  )

  (func (export "actrail_observation_consume") (param $ptr i32) (param $len i32) (result i64)
    (if
      (i32.or
        (i32.eqz (global.get $config_len))
        (i32.eqz (local.get $len))
      )
      (then (return (i64.const -1)))
    )
    (i64.const -31)
  )
)
