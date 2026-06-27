(component
  (type $observation_event_family
    (enum
      "semantic-action"
      "semantic-action-link"
      "diagnostic"
      "trace-lifecycle"
      "resource-metric"
      "payload-metadata"))
  (type $semantic_action_record
    (record
      (field "trace-id" string)
      (field "action-id" string)
      (field "kind" string)
      (field "summary" string)))
  (type $payload_ref
    (record
      (field "id" string)
      (field "trace-id" string)))
  (type $batch
    (record
      (field "trace-id" string)
      (field "families" (list $observation_event_family))
      (field "semantic-actions" (list $semantic_action_record))
      (field "payload-refs" (list $payload_ref))))
  (type $report
    (record
      (field "observed-records" u64)
      (field "dropped-records" u64)))
  (type $consume_ty
    (func
      (param "batch" $batch)
      (result (result $report (error string)))))

  (core module $m
    (memory (export "memory") 1)
    (global $next (mut i32) (i32.const 4096))

    (func (export "realloc")
      (param $old_ptr i32)
      (param $old_len i32)
      (param $align i32)
      (param $new_len i32)
      (result i32)
      (local $ptr i32)
      (local.set $ptr
        (i32.and
          (i32.add
            (global.get $next)
            (i32.sub (local.get $align) (i32.const 1)))
          (i32.xor
            (i32.sub (local.get $align) (i32.const 1))
            (i32.const -1))))
      (global.set $next
        (i32.add
          (local.get $ptr)
          (local.get $new_len)))
      (local.get $ptr))

    (func (export "consume")
      (param $trace_ptr i32)
      (param $trace_len i32)
      (param $families_ptr i32)
      (param $families_len i32)
      (param $actions_ptr i32)
      (param $actions_len i32)
      (param $payload_refs_ptr i32)
      (param $payload_refs_len i32)
      (result i32)
      (i32.store (i32.const 0) (i32.const 0))
      (i64.store (i32.const 8) (i64.const 1))
      (i64.store (i32.const 16) (i64.const 0))
      (i32.const 0))

    (func (export "post-return") (param $ptr i32))
  )

  (core instance $i (instantiate $m))
  (func $consume (type $consume_ty)
    (canon lift
      (core func $i "consume")
      (memory $i "memory")
      (realloc (func $i "realloc"))
      (post-return (func $i "post-return"))))

  (instance $obs
    (export "observation-event-family" (type $observation_event_family))
    (export "semantic-action-record" (type $semantic_action_record))
    (export "payload-ref" (type $payload_ref))
    (export "observation-batch" (type $batch))
    (export "observation-report" (type $report))
    (export "consume" (func $consume)))
  (export "actrail:plugin/observation-consumer@0.1.0" (instance $obs))
)
