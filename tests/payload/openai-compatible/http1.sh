set -euo pipefail

exec bash "$(dirname "$0")/request.sh" http1.1 "Hello!"
