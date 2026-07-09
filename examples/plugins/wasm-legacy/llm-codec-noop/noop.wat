(module
  (memory (export "memory") 1)
  (global $heap (mut i32) (i32.const 1024))
  (data (i32.const 16) "{\"status\":\"no_match\"}")
  (func (export "actrail_alloc") (param $len i32) (result i32)
    (local $ptr i32)
    global.get $heap
    local.set $ptr
    global.get $heap
    local.get $len
    i32.add
    global.set $heap
    local.get $ptr)
  (func (export "actrail_llm_codec_decode") (param $ptr i32) (param $len i32) (result i64)
    i64.const 68719476757))
