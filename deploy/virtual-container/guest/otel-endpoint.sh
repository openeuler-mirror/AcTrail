#!/usr/bin/env bash
# Shared validation and rendering for the Guest OTLP/HTTP exporter endpoint.
#
# The Guest reaches the Collector through exactly one of two egress modes, and
# they differ only in which destination address is legitimate:
#
#   network       CNI/Kubernetes gives the Guest its own interface and route, so
#                 the endpoint must name a host address reachable from inside the
#                 Guest. Guest loopback is the Guest itself and stays rejected.
#   vsock-bridge  There is no Guest network. A loopback listener inside the Guest
#                 forwards over AF_VSOCK to the host, so Guest loopback is the
#                 only legitimate destination, and its port doubles as the single
#                 source of truth for the bridge listen port.
#
# Every other rule — scheme, traces path, placeholder, IPv4 canonicalisation,
# port range, TOML safety — applies identically to both modes.

ACTRAIL_GUEST_OTEL_ENDPOINT_ERROR=""
# Explicit port of the last validated endpoint; empty when none was given.
ACTRAIL_GUEST_OTEL_ENDPOINT_PORT=""
ACTRAIL_GUEST_OTEL_EGRESS_MODE_DEFAULT="network"

actrail_guest_otel_endpoint_reject() {
  ACTRAIL_GUEST_OTEL_ENDPOINT_ERROR="$1"
}

actrail_validate_guest_egress_mode() {
  local mode="${1-}"

  case "$mode" in
    network|vsock-bridge)
      return 0
      ;;
    *)
      actrail_guest_otel_endpoint_reject \
        "unsupported egress mode ${mode:-<empty>}: expected network or vsock-bridge"
      return 1
      ;;
  esac
}

# Validate the optional exporter selection used by deployment entry points.
# The low-level endpoint validator intentionally keeps rejecting an empty value
# so callers that render an exporter configuration cannot accidentally install
# a placeholder. An omitted endpoint selects local SQLite-only operation.
actrail_validate_guest_otel_selection() {
  local endpoint="${1-}"
  local mode="$ACTRAIL_GUEST_OTEL_EGRESS_MODE_DEFAULT"

  if [[ "$#" -ge 2 ]]; then
    mode="$2"
  fi
  actrail_validate_guest_egress_mode "$mode" || return 1
  if [[ -z "$endpoint" ]]; then
    if [[ "$mode" == "vsock-bridge" ]]; then
      actrail_guest_otel_endpoint_reject \
        "--egress-mode vsock-bridge requires --otel-endpoint"
      return 1
    fi
    ACTRAIL_GUEST_OTEL_ENDPOINT_ERROR=""
    ACTRAIL_GUEST_OTEL_ENDPOINT_PORT=""
    return 0
  fi
  actrail_validate_guest_otel_endpoint "$endpoint" "$mode"
}

actrail_validate_guest_otel_endpoint() {
  local endpoint="${1-}"
  local mode="$ACTRAIL_GUEST_OTEL_EGRESS_MODE_DEFAULT"
  local normalized_endpoint=""
  local remainder=""
  local authority=""
  local path=""
  local host=""
  local normalized_host=""
  local port=""
  local explicit_port=0
  local port_number=0
  local is_loopback_literal=0
  local -a ipv4_octets=()
  local -a dns_labels=()
  local octet=""
  local label=""

  ACTRAIL_GUEST_OTEL_ENDPOINT_ERROR=""
  ACTRAIL_GUEST_OTEL_ENDPOINT_PORT=""
  # An omitted mode keeps the historical network-egress behaviour; an explicitly
  # passed empty mode is a caller bug and must not be silently defaulted.
  if [[ "$#" -ge 2 ]]; then
    mode="$2"
  fi
  actrail_validate_guest_egress_mode "$mode" || return 1
  if [[ -z "$endpoint" ]]; then
    actrail_guest_otel_endpoint_reject "--otel-endpoint is required"
    return 1
  fi
  case "$endpoint" in
    http://*)
      remainder="${endpoint#http://}"
      ;;
    https://*)
      remainder="${endpoint#https://}"
      ;;
    *)
      actrail_guest_otel_endpoint_reject \
        "--otel-endpoint must start with http:// or https://"
      return 1
      ;;
  esac

  normalized_endpoint="${endpoint,,}"
  case "$normalized_endpoint" in
    *collector_host*|*collector-host*|*placeholder*|*replace_me*|*replace-me*|*change_me*|*change-me*)
      actrail_guest_otel_endpoint_reject \
        "--otel-endpoint must be a concrete address, not a placeholder"
      return 1
      ;;
  esac
  if [[ "$endpoint" == *'"'* || "$endpoint" == *'\'* \
    || "$endpoint" == *[[:cntrl:]]* || "$endpoint" =~ [[:space:]] ]]; then
    actrail_guest_otel_endpoint_reject \
      "--otel-endpoint contains characters that are unsafe in the Guest TOML config"
    return 1
  fi
  if [[ "$endpoint" == *'?'* || "$endpoint" == *'#'* ]]; then
    actrail_guest_otel_endpoint_reject \
      "--otel-endpoint must not contain a query or fragment"
    return 1
  fi
  if [[ "$remainder" != */* ]]; then
    actrail_guest_otel_endpoint_reject \
      "--otel-endpoint must include a traces path ending in /v1/traces"
    return 1
  fi

  authority="${remainder%%/*}"
  path="${remainder#"$authority"}"
  if [[ -z "$authority" ]]; then
    actrail_guest_otel_endpoint_reject "--otel-endpoint must include a host"
    return 1
  fi
  if [[ "$path" != */v1/traces ]]; then
    actrail_guest_otel_endpoint_reject \
      "--otel-endpoint traces path must end in /v1/traces"
    return 1
  fi
  if [[ "$authority" == *@* ]]; then
    actrail_guest_otel_endpoint_reject \
      "--otel-endpoint must not contain userinfo; use the supported TLS configuration"
    return 1
  fi

  if [[ "$authority" == \[* ]]; then
    actrail_guest_otel_endpoint_reject \
      "--otel-endpoint IPv6 literals are not supported by the current exporter TLS verifier"
    return 1
  elif [[ "$authority" == *:* ]]; then
    if [[ "$authority" == *:*:* ]]; then
      actrail_guest_otel_endpoint_reject \
        "--otel-endpoint IPv6 hosts must be bracketed"
      return 1
    fi
    host="${authority%:*}"
    port="${authority##*:}"
    explicit_port=1
  else
    host="$authority"
  fi

  if [[ "$host" =~ ^[0-9.]+$ ]]; then
    # Reject inet_aton-style shorthand/octal/integer spellings. Different
    # resolvers can interpret values such as 2130706433 or 0177.0.0.1 as
    # 127.0.0.1, which would silently point back into the Guest.
    if [[ ! "$host" =~ ^[0-9]{1,3}(\.[0-9]{1,3}){3}$ ]]; then
      actrail_guest_otel_endpoint_reject \
        "--otel-endpoint contains an invalid or ambiguous IPv4 address; use canonical dotted-decimal IPv4"
      return 1
    fi
    IFS='.' read -r -a ipv4_octets <<<"$host"
    for octet in "${ipv4_octets[@]}"; do
      if [[ ${#octet} -gt 1 && "$octet" == 0* ]] || (( 10#$octet > 255 )); then
        actrail_guest_otel_endpoint_reject \
          "--otel-endpoint contains an invalid or ambiguous IPv4 address"
        return 1
      fi
    done
  else
    if [[ ! "$host" =~ ^[A-Za-z0-9._-]+$ ]]; then
      actrail_guest_otel_endpoint_reject "--otel-endpoint contains an invalid host"
      return 1
    fi
    IFS='.' read -r -a dns_labels <<<"$host"
    for label in "${dns_labels[@]}"; do
      if [[ -z "$label" \
        || ! "$label" =~ ^[A-Za-z0-9]([A-Za-z0-9_-]*[A-Za-z0-9])?$ ]]; then
        actrail_guest_otel_endpoint_reject "--otel-endpoint contains an invalid host"
        return 1
      fi
    done
  fi
  if (( explicit_port )); then
    if [[ ! "$port" =~ ^[0-9]{1,5}$ ]]; then
      actrail_guest_otel_endpoint_reject \
        "--otel-endpoint port must be an integer between 1 and 65535"
      return 1
    fi
    port_number=$((10#$port))
    if (( port_number < 1 || port_number > 65535 )); then
      actrail_guest_otel_endpoint_reject \
        "--otel-endpoint port must be an integer between 1 and 65535"
      return 1
    fi
    ACTRAIL_GUEST_OTEL_ENDPOINT_PORT="$port_number"
  fi

  normalized_host="${host,,}"
  # Only a literal 127.0.0.0/8 address counts as the bridge listener. "localhost"
  # depends on Guest DNS and /etc/hosts, which the deployment does not control.
  if [[ ${#ipv4_octets[@]} -eq 4 && ${ipv4_octets[0]} -eq 127 ]]; then
    is_loopback_literal=1
  fi

  if [[ "$mode" == "vsock-bridge" ]]; then
    if (( ! is_loopback_literal )); then
      actrail_guest_otel_endpoint_reject \
        "vsock-bridge egress requires the Guest bridge loopback address (127.0.0.0/8 literal)"
      return 1
    fi
    if (( ! explicit_port )); then
      actrail_guest_otel_endpoint_reject \
        "vsock-bridge egress requires an explicit port; it is also the Guest bridge listen port"
      return 1
    fi
    return 0
  fi

  if [[ "$normalized_host" == "localhost" \
    || "$normalized_host" == localhost.* \
    || $is_loopback_literal -eq 1 \
    || "$normalized_host" == "0.0.0.0" \
    || "$normalized_host" == "[::1]" \
    || "$normalized_host" == "[0:0:0:0:0:0:0:1]" ]]; then
    actrail_guest_otel_endpoint_reject \
      "--otel-endpoint must name a host reachable from the Guest; Guest loopback is not the host"
    return 1
  fi

  return 0
}

# Echo the explicit port of a VSOCK-bridge endpoint. The endpoint is the single
# source of truth for the Guest bridge listen port, so the installer renders the
# unit from this value instead of repeating a second constant.
actrail_guest_otel_endpoint_port() {
  local endpoint="${1-}"
  local mode="vsock-bridge"

  if [[ "$#" -ge 2 ]]; then
    mode="$2"
  fi
  actrail_validate_guest_otel_endpoint "$endpoint" "$mode" || return 1
  if [[ -z "$ACTRAIL_GUEST_OTEL_ENDPOINT_PORT" ]]; then
    actrail_guest_otel_endpoint_reject \
      "--otel-endpoint must include an explicit port to derive the bridge listen port"
    return 1
  fi
  printf '%s\n' "$ACTRAIL_GUEST_OTEL_ENDPOINT_PORT"
}

actrail_write_guest_otel_endpoint_config() {
  local template="${1-}"
  local target="${2-}"
  local endpoint="${3-}"
  local mode="$ACTRAIL_GUEST_OTEL_EGRESS_MODE_DEFAULT"
  local temp_path=""

  if [[ "$#" -ge 4 ]]; then
    mode="$4"
  fi
  ACTRAIL_GUEST_OTEL_ENDPOINT_ERROR=""
  if ! actrail_validate_guest_otel_endpoint "$endpoint" "$mode"; then
    return 1
  fi
  if [[ ! -f "$template" ]]; then
    actrail_guest_otel_endpoint_reject "otel-http config template not found: $template"
    return 1
  fi
  if [[ -z "$target" || ! -d "$(dirname -- "$target")" ]]; then
    actrail_guest_otel_endpoint_reject \
      "otel-http config target directory does not exist: $target"
    return 1
  fi
  if ! temp_path="$(mktemp "${target}.tmp.XXXXXX")"; then
    actrail_guest_otel_endpoint_reject \
      "cannot create temporary otel-http config beside: $target"
    return 1
  fi
  if ! awk -v endpoint="$endpoint" '
      $0 == "# Replace the placeholder host before deployment. Production deployments" {
        next
      }
      $0 == "# should use https:// and configure the three TLS paths below." {
        next
      }
      /^[[:space:]]*endpoint[[:space:]]*=/ {
        endpoint_count++
        if (endpoint_count == 1) {
          print "# Collector endpoint explicitly injected during Guest image deployment."
          printf "endpoint = \"%s\"\n", endpoint
        }
        next
      }
      { print }
      END { if (endpoint_count != 1) exit 42 }
    ' "$template" >"$temp_path"; then
    rm -f -- "$temp_path"
    actrail_guest_otel_endpoint_reject \
      "otel-http config template must contain exactly one endpoint assignment"
    return 1
  fi
  if ! install -m 0640 -- "$temp_path" "$target"; then
    rm -f -- "$temp_path"
    actrail_guest_otel_endpoint_reject "cannot install otel-http config: $target"
    return 1
  fi
  rm -f -- "$temp_path"
  if ! grep -Fqx -- "endpoint = \"$endpoint\"" "$target"; then
    actrail_guest_otel_endpoint_reject \
      "installed otel-http config does not contain the requested endpoint"
    return 1
  fi
  return 0
}
