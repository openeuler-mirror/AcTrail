set -euo pipefail

mode="${1:-run}"
case "$mode" in
  run|prepare) ;;
  *)
    echo "unsupported mode: $mode" >&2
    exit 2
    ;;
esac

exec bash "$(dirname "$0")/request.sh" "$mode" http1.1 "Hello!"
