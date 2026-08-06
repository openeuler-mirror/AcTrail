#!/usr/bin/env bash
# Shared validation and rendering for the Guest OTLP/HTTP exporter endpoint.

ACTRAIL_GUEST_OTEL_ENDPOINT_ERROR=""

actrail_guest_otel_endpoint_reject() {
  ACTRAIL_GUEST_OTEL_ENDPOINT_ERROR="$1"
}

actrail_validate_guest_otel_endpoint() {
  local endpoint="${1-}"
  local normalized_endpoint=""
  local remainder=""
  local authority=""
  local path=""
  local host=""
  local normalized_host=""
  local port=""
  local explicit_port=0
  local port_number=0
  local -a ipv4_octets=()
  local -a dns_labels=()
  local octet=""
  local label=""

  ACTRAIL_GUEST_OTEL_ENDPOINT_ERROR=""
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
  fi

  normalized_host="${host,,}"
  if [[ "$normalized_host" == "localhost" \
    || "$normalized_host" == localhost.* \
    || ( ${#ipv4_octets[@]} -eq 4 && ${ipv4_octets[0]} -eq 127 ) \
    || "$normalized_host" == "0.0.0.0" \
    || "$normalized_host" == "[::1]" \
    || "$normalized_host" == "[0:0:0:0:0:0:0:1]" ]]; then
    actrail_guest_otel_endpoint_reject \
      "--otel-endpoint must name a host reachable from the Guest; Guest loopback is not the host"
    return 1
  fi

  return 0
}

actrail_write_guest_otel_endpoint_config() {
  local template="${1-}"
  local target="${2-}"
  local endpoint="${3-}"
  local temp_path=""

  ACTRAIL_GUEST_OTEL_ENDPOINT_ERROR=""
  if ! actrail_validate_guest_otel_endpoint "$endpoint"; then
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
