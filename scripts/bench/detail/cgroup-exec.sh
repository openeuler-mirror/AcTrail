#!/bin/sh
set -eu

membership_file=$1
shift
printf '%s\n' "$$" > "$membership_file"
exec "$@"
