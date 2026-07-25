#!/bin/sh
set -eu

exec /root/.cargo/bin/xiaoo --cli run \
    --config "$ACTRAIL_XIAOO_CONFIG" \
    --tools bash \
    --max-turns 3 \
    --prompt "$ACTRAIL_XIAOO_PROMPT"
