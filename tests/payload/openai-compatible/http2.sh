set -euo pipefail

exec bash "$(dirname "$0")/request.sh" http2 "Hello over HTTP/2!"
