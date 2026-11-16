(module
  (import "actrail_host" "file_access_current_match_get"
    (func $file_access_current_match_get (param i32 i32 i32 i32 i32 i32) (result i64)))

  (memory (export "memory") 1)
  (global $next (mut i32) (i32.const 4096))

  (data (i32.const 1024) "f")
  (data (i32.const 1056) "matched-rule.v1")

  (func (export "actrail_alloc") (param $len i32) (result i32)
    (local $ptr i32)
    (local.set $ptr (global.get $next))
    (global.set $next (i32.add (global.get $next) (i32.add (local.get $len) (i32.const 8))))
    (local.get $ptr)
  )

  (func (export "actrail_plugin_init") (param $ptr i32) (param $len i32) (result i32)
    (i32.const 0)
  )

  (func (export "actrail_control_decide") (param $ptr i32) (param $len i32) (result i64)
    (local $read i64)
    (local.set $read
      (call $file_access_current_match_get
        (i32.const 1024)
        (i32.const 1)
        (i32.const 1056)
        (i32.const 15)
        (i32.const 2048)
        (i32.const 512)
      )
    )
    (if (i64.le_s (local.get $read) (i64.const 0))
      (then (return (i64.const -1)))
    )
    (i64.const 1)
  )
)
