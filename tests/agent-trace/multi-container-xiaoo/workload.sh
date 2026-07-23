#!/bin/sh
set -eu

dd if="$ACTRAIL_TASK_INPUT" of="$ACTRAIL_TASK_OUTPUT" bs=4096 count=1 2>/dev/null
printf '%s\n' "$ACTRAIL_TASK_WRITE_MARKER" >>"$ACTRAIL_TASK_OUTPUT"
dd if="$ACTRAIL_TASK_OUTPUT" of=/dev/null bs=4096 count=1 2>/dev/null

if [ "$ACTRAIL_TASK_HOLD_SECONDS" != "0" ]; then
    sleep "$ACTRAIL_TASK_HOLD_SECONDS"
fi

exec /root/.cargo/bin/xiaoo --cli run \
    --config "$ACTRAIL_XIAOO_CONFIG" \
    --no-tools \
    --max-turns 1 \
    --prompt "$ACTRAIL_XIAOO_PROMPT"
