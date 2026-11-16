(module
  (import "actrail_host" "context_query"
    (func $context_query (param i32 i32 i32 i32 i32 i32) (result i64)))

  (memory (export "memory") 1)
  (global $next (mut i32) (i32.const 4096))

  (data (i32.const 1024) "c")
  (data (i32.const 1056) "decision-summary.v1")

  (func (export "actrail_alloc") (param $len i32) (result i32)
    (local $ptr i32)
    (local.set $ptr (global.get $next))
    (global.set $next (i32.add (global.get $next) (i32.add (local.get $len) (i32.const 8))))
    (local.get $ptr)
  )

  (func (export "actrail_plugin_init") (param $ptr i32) (param $len i32) (result i32)
    (i32.const 0)
  )

  (func $has_current_decision_view (result i32)
    (if (i32.ne (i32.load8_u (i32.const 2048)) (i32.const 118)) (then (return (i32.const 0)))) ;; v
    (if (i32.ne (i32.load8_u (i32.const 2055)) (i32.const 61)) (then (return (i32.const 0)))) ;; =
    (if (i32.ne (i32.load8_u (i32.const 2056)) (i32.const 99)) (then (return (i32.const 0)))) ;; c
    (if (i32.ne (i32.load8_u (i32.const 2072)) (i32.const 10)) (then (return (i32.const 0)))) ;; \n
    (if (i32.ne (i32.load8_u (i32.const 2073)) (i32.const 115)) (then (return (i32.const 0)))) ;; s
    (if (i32.ne (i32.load8_u (i32.const 2080)) (i32.const 61)) (then (return (i32.const 0)))) ;; =
    (if (i32.ne (i32.load8_u (i32.const 2081)) (i32.const 102)) (then (return (i32.const 0)))) ;; f
    (if (i32.ne (i32.load8_u (i32.const 2092)) (i32.const 10)) (then (return (i32.const 0)))) ;; \n
    (if (i32.ne (i32.load8_u (i32.const 2093)) (i32.const 111)) (then (return (i32.const 0)))) ;; o
    (if (i32.ne (i32.load8_u (i32.const 2102)) (i32.const 61)) (then (return (i32.const 0)))) ;; =
    (if (i32.ne (i32.load8_u (i32.const 2103)) (i32.const 111)) (then (return (i32.const 0)))) ;; o
    (if (i32.ne (i32.load8_u (i32.const 2107)) (i32.const 10)) (then (return (i32.const 0)))) ;; \n
    (if (i32.ne (i32.load8_u (i32.const 2108)) (i32.const 116)) (then (return (i32.const 0)))) ;; t
    (if (i32.ne (i32.load8_u (i32.const 2122)) (i32.const 61)) (then (return (i32.const 0)))) ;; =
    (i32.const 1)
  )

  (func (export "actrail_control_decide") (param $ptr i32) (param $len i32) (result i64)
    (local $read i64)
    (local.set $read
      (call $context_query
        (i32.const 1024)
        (i32.const 1)
        (i32.const 1056)
        (i32.const 19)
        (i32.const 2048)
        (i32.const 512)
      )
    )
    (if (i64.lt_s (local.get $read) (i64.const 70))
      (then (return (i64.const -1)))
    )
    (if (result i64)
      (call $has_current_decision_view)
      (then (i64.const 1))
      (else (i64.const -1))
    )
  )
)
