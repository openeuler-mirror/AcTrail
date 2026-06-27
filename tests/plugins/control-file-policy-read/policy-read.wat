(module
  (import "actrail_host" "file_policy_read"
    (func $file_policy_read (param i32 i32 i32 i32 i32 i32) (result i64)))

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

  (func $has_gray_rule_line (result i32)
    (if (i32.ne (i32.load8_u (i32.const 2076)) (i32.const 114)) (then (return (i32.const 0)))) ;; r
    (if (i32.ne (i32.load8_u (i32.const 2077)) (i32.const 117)) (then (return (i32.const 0)))) ;; u
    (if (i32.ne (i32.load8_u (i32.const 2078)) (i32.const 108)) (then (return (i32.const 0)))) ;; l
    (if (i32.ne (i32.load8_u (i32.const 2079)) (i32.const 101)) (then (return (i32.const 0)))) ;; e
    (if (i32.ne (i32.load8_u (i32.const 2080)) (i32.const 95)) (then (return (i32.const 0)))) ;; _
    (if (i32.ne (i32.load8_u (i32.const 2081)) (i32.const 105)) (then (return (i32.const 0)))) ;; i
    (if (i32.ne (i32.load8_u (i32.const 2082)) (i32.const 100)) (then (return (i32.const 0)))) ;; d
    (if (i32.ne (i32.load8_u (i32.const 2083)) (i32.const 61)) (then (return (i32.const 0)))) ;; =
    (if (i32.ne (i32.load8_u (i32.const 2084)) (i32.const 103)) (then (return (i32.const 0)))) ;; g
    (if (i32.ne (i32.load8_u (i32.const 2085)) (i32.const 114)) (then (return (i32.const 0)))) ;; r
    (if (i32.ne (i32.load8_u (i32.const 2086)) (i32.const 97)) (then (return (i32.const 0)))) ;; a
    (if (i32.ne (i32.load8_u (i32.const 2087)) (i32.const 121)) (then (return (i32.const 0)))) ;; y
    (if (i32.ne (i32.load8_u (i32.const 2088)) (i32.const 45)) (then (return (i32.const 0)))) ;; -
    (if (i32.ne (i32.load8_u (i32.const 2089)) (i32.const 102)) (then (return (i32.const 0)))) ;; f
    (if (i32.ne (i32.load8_u (i32.const 2090)) (i32.const 105)) (then (return (i32.const 0)))) ;; i
    (if (i32.ne (i32.load8_u (i32.const 2091)) (i32.const 108)) (then (return (i32.const 0)))) ;; l
    (if (i32.ne (i32.load8_u (i32.const 2092)) (i32.const 101)) (then (return (i32.const 0)))) ;; e
    (i32.const 1)
  )

  (func $has_required_policy_view (result i32)
    (if (i32.eqz (call $has_gray_rule_line)) (then (return (i32.const 0))))
    (if (i32.ne (i32.load8_u (i32.const 2094)) (i32.const 100)) (then (return (i32.const 0)))) ;; d
    (if (i32.ne (i32.load8_u (i32.const 2102)) (i32.const 61)) (then (return (i32.const 0)))) ;; =
    (if (i32.ne (i32.load8_u (i32.const 2103)) (i32.const 103)) (then (return (i32.const 0)))) ;; g
    (if (i32.ne (i32.load8_u (i32.const 2107)) (i32.const 10)) (then (return (i32.const 0)))) ;; \n
    (if (i32.ne (i32.load8_u (i32.const 2108)) (i32.const 102)) (then (return (i32.const 0)))) ;; f
    (if (i32.ne (i32.load8_u (i32.const 2116)) (i32.const 61)) (then (return (i32.const 0)))) ;; =
    (if (i32.ne (i32.load8_u (i32.const 2117)) (i32.const 100)) (then (return (i32.const 0)))) ;; d
    (if (i32.ne (i32.load8_u (i32.const 2121)) (i32.const 10)) (then (return (i32.const 0)))) ;; \n
    (if (i32.ne (i32.load8_u (i32.const 2122)) (i32.const 116)) (then (return (i32.const 0)))) ;; t
    (if (i32.ne (i32.load8_u (i32.const 2132)) (i32.const 61)) (then (return (i32.const 0)))) ;; =
    (if (i32.ne (i32.load8_u (i32.const 2133)) (i32.const 53)) (then (return (i32.const 0)))) ;; 5
    (if (i32.ne (i32.load8_u (i32.const 2137)) (i32.const 10)) (then (return (i32.const 0)))) ;; \n
    (if (i32.ne (i32.load8_u (i32.const 2138)) (i32.const 99)) (then (return (i32.const 0)))) ;; c
    (if (i32.ne (i32.load8_u (i32.const 2155)) (i32.const 61)) (then (return (i32.const 0)))) ;; =
    (if (i32.ne (i32.load8_u (i32.const 2156)) (i32.const 49)) (then (return (i32.const 0)))) ;; 1
    (if (i32.ne (i32.load8_u (i32.const 2157)) (i32.const 10)) (then (return (i32.const 0)))) ;; \n
    (i32.const 1)
  )

  (func (export "actrail_control_decide") (param $ptr i32) (param $len i32) (result i64)
    (local $read i64)
    (local.set $read
      (call $file_policy_read
        (i32.const 1024)
        (i32.const 1)
        (i32.const 1056)
        (i32.const 15)
        (i32.const 2048)
        (i32.const 512)
      )
    )
    (if (i64.lt_s (local.get $read) (i64.const 110))
      (then (return (i64.const -1)))
    )
    (if (result i64)
      (call $has_required_policy_view)
      (then (i64.const 1))
      (else (i64.const -1))
    )
  )
)
