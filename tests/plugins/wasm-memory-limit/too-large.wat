(module
  ;; Two pages require 131072 bytes. The manifest caps this plugin at one page.
  (memory (export "memory") 2)

  (func (export "actrail_alloc") (param $len i32) (result i32)
    (i32.const 0)
  )

  (func (export "actrail_control_decide") (param $ptr i32) (param $len i32) (result i64)
    (i64.const 1)
  )
)
