#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 5 ]; then
  echo "usage: script_b.sh <provider> <model> <max-turns> <no-tools> <prompt>" >&2
  exit 2
fi

provider=$1
model=$2
max_turns=$3
no_tools=$4
prompt=$5

args=(run --provider "$provider" --model "$model" --max-turns "$max_turns")
if [ "$no_tools" = "true" ]; then
  args+=(--no-tools)
elif [ "$no_tools" != "false" ]; then
  echo "invalid no-tools value: $no_tools" >&2
  exit 2
fi
args+=(-p "$prompt")

xiaoo "${args[@]}"
