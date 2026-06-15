#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/package-source.sh --output <tar.gz> [--version <version>] [--tree-ish <ref>]

Creates an AcTrail source tarball with prebuilt actrailweb frontend assets
and the embedded Java payload agent jar.
The version defaults to the workspace package version in Cargo.toml.
The tree-ish defaults to HEAD.
Requires npm and JDK 17+ on PATH.
ACTRAIL_JAVA_AGENT_RELEASE overrides the javac --release target and defaults to 17.
USAGE
}

output_path=""
version=""
treeish="HEAD"
caller_dir="$PWD"

while [ "$#" -gt 0 ]; do
  case "$1" in
    --output)
      output_path="${2:?missing value for --output}"
      shift 2
      ;;
    --version)
      version="${2:?missing value for --version}"
      shift 2
      ;;
    --tree-ish)
      treeish="${2:?missing value for --tree-ish}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [ -z "$output_path" ]; then
  echo "missing required --output <tar.gz>" >&2
  usage >&2
  exit 2
fi

case "$output_path" in
  /*) ;;
  *) output_path="$caller_dir/$output_path" ;;
esac

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

if [ -z "$version" ]; then
  version="$(
    awk '
      $0 == "[workspace.package]" { in_workspace_package = 1; next }
      /^\[/ { in_workspace_package = 0 }
      in_workspace_package && $1 == "version" {
        gsub(/"/, "", $3)
        print $3
        exit
      }
    ' Cargo.toml
  )"
fi

if [ -z "$version" ]; then
  echo "failed to read workspace package version from Cargo.toml" >&2
  exit 1
fi

package_dir="AcTrail-${version}"
staging_dir="$(mktemp -d "${TMPDIR:-/tmp}/actrail-source.XXXXXXXX")"
cleanup() {
  rm -rf "$staging_dir"
}
trap cleanup EXIT

archive_path="$staging_dir/source.tar"
git archive --format=tar --prefix="${package_dir}/" -o "$archive_path" "$treeish"
tar -xf "$archive_path" -C "$staging_dir"

package_root="$staging_dir/$package_dir"
frontend_dir="$package_root/crates/apps/web/frontend"
frontend_dist="$frontend_dir/dist"
java_agent_dir="$package_root/crates/apps/ctl/java-agent"
java_agent_source_dir="$java_agent_dir/src/main/java"
java_agent_dist="$java_agent_dir/dist"
java_agent_jar="$java_agent_dist/actrail-java-payload-agent.jar"
java_agent_build_dir="$staging_dir/java-agent-build"
java_agent_classes="$java_agent_build_dir/classes"
java_agent_manifest="$java_agent_build_dir/MANIFEST.MF"
java_agent_release="${ACTRAIL_JAVA_AGENT_RELEASE:-17}"

npm ci --prefix "$frontend_dir"
npm run build --prefix "$frontend_dir" -- --outDir "$frontend_dist"

mapfile -t java_agent_sources < <(find "$java_agent_source_dir" -type f -name '*.java' | sort)
if [ "${#java_agent_sources[@]}" -eq 0 ]; then
  echo "no Java agent sources under $java_agent_source_dir" >&2
  exit 1
fi

rm -rf "$java_agent_dist"
mkdir -p "$java_agent_dist" "$java_agent_classes"
cat > "$java_agent_manifest" <<'MANIFEST'
Manifest-Version: 1.0
Premain-Class: com.actrail.javaagent.AcTrailJavaPayloadAgent
Can-Redefine-Classes: true
Can-Retransform-Classes: true

MANIFEST
javac --release "$java_agent_release" -d "$java_agent_classes" "${java_agent_sources[@]}"
jar cfm "$java_agent_jar" "$java_agent_manifest" -C "$java_agent_classes" .

mkdir -p "$(dirname "$output_path")"
tar --exclude="${package_dir}/crates/apps/web/frontend/node_modules" \
  -czf "$output_path" \
  -C "$staging_dir" \
  "$package_dir"

echo "$output_path"
