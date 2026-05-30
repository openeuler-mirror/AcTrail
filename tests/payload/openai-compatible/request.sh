set -euo pipefail

http_version="${1:?missing curl HTTP version}"
default_prompt="${2:?missing default prompt}"

case "$http_version" in
  http1.1|http2) ;;
  *)
    echo "unsupported curl HTTP version: $http_version" >&2
    exit 2
    ;;
esac

base_url="${ACTRAIL_LLM_BASE_URL:-https://api.deepseek.com}"
chat_path="${ACTRAIL_LLM_CHAT_PATH:-/chat/completions}"
model="${ACTRAIL_LLM_MODEL:-deepseek-v4-pro}"
api_key_env="${ACTRAIL_LLM_API_KEY_ENV:-DEEPSEEK_API_KEY}"
api_key="${!api_key_env:-}"

if [ -z "$api_key" ]; then
  echo "missing environment variable $api_key_env" >&2
  exit 2
fi

if [[ "$chat_path" != /* ]]; then
  echo "ACTRAIL_LLM_CHAT_PATH must start with /" >&2
  exit 2
fi

auth_header_name="${ACTRAIL_LLM_AUTH_HEADER_NAME:-Authorization}"
auth_scheme="${ACTRAIL_LLM_AUTH_SCHEME-Bearer}"
if [ -n "$auth_scheme" ]; then
  auth_header="$auth_header_name: $auth_scheme $api_key"
else
  auth_header="$auth_header_name: $api_key"
fi

export ACTRAIL_LLM_MODEL="$model"
export ACTRAIL_LLM_PROMPT="${ACTRAIL_LLM_PROMPT:-$default_prompt}"

request_json="${ACTRAIL_LLM_REQUEST_JSON:-}"
if [ -z "$request_json" ]; then
  request_json="$(
    python3 - <<'PY'
import json
import os

body = {
    "model": os.environ["ACTRAIL_LLM_MODEL"],
    "messages": [
        {"role": "system", "content": "You are a helpful assistant."},
        {"role": "user", "content": os.environ["ACTRAIL_LLM_PROMPT"]},
    ],
    "stream": False,
}
print(json.dumps(body, ensure_ascii=False, separators=(",", ":")))
PY
  )"
fi

body_file="$(mktemp)"
trap 'rm -f "$body_file"' EXIT
printf '%s' "$request_json" >"$body_file"

curl --config - --data-binary @"$body_file" <<EOF
$http_version
url = "${base_url%/}${chat_path}"
header = "Content-Type: application/json"
header = "$auth_header"
EOF
