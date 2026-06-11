#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/package-source.sh --output <tar.gz> [--version <version>] [--tree-ish <ref>]

Creates an AcTrail source tarball with prebuilt actrailweb frontend assets.
The version defaults to the workspace package version in Cargo.toml.
The tree-ish defaults to HEAD.
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

npm ci --prefix "$frontend_dir"
npm run build --prefix "$frontend_dir" -- --outDir "$frontend_dist"

mkdir -p "$(dirname "$output_path")"
tar --exclude="${package_dir}/crates/apps/web/frontend/node_modules" \
  -czf "$output_path" \
  -C "$staging_dir" \
  "$package_dir"

echo "$output_path"
