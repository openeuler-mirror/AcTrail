(module
  (memory (export "memory") 1)
  (global $next (mut i32) (i32.const 1024))

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
    ;; Deny-once. The E2E verifies AcTrail only calls this plugin for an
    ;; explicit command policy match, not for every exec.
    (i64.const -1)
  )
)
