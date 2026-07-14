#!/usr/bin/env bash
set -Eeuo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/append-patch.sh --target-dir <rpm-source-dir> --name <patch-name> [options]

Generates an incremental Git-format RPM patch against Source0 plus the patches
already listed in the spec, copies the patch into the RPM source directory, and
appends a PatchNNNN line to the spec file by default.

Required:
  --target-dir <dir>       RPM source package directory, for example ../src-AcTrail
  --name <patch-name>      Patch filename slug. Use letters, numbers, dot, plus,
                           underscore, or hyphen. The .patch suffix is optional.

Options:
  --source-dir <dir>       Source git repository. Defaults to this script's repo.
  --tree-ish <ref>         Git tree to package. Defaults to HEAD.
  --working-tree           Use tracked files from the source working tree instead
                           of git archive. Untracked files are not included.
  --base-tar <tar.gz>      Source0 tarball. Defaults to Source0 from the spec.
  --spec <path>            Spec file. Defaults to the single *.spec in target dir.
  --patch-index <n>        Patch index. Defaults to the next index from spec and
                           existing NNNN-*.patch files, starting at 1.
  --no-spec-update         Only write the patch file; do not edit the spec.
  --preserve-path <path>   Preserve a path from Source0 when it is absent from the
                           new tree. May be repeated.
  --no-default-preserve    Disable the default preserve paths.
  -h, --help               Show this help.

Default preserve paths:
  crates/apps/web/frontend/dist
  crates/apps/ctl/java-agent/dist

The generated patch uses a/... and b/... paths and carries binary data plus file
mode changes. When the spec is updated, the script ensures %autosetup uses
`-S git -p1`; the RPM package must declare `BuildRequires: git`.
USAGE
}

fail() {
  echo "error: $*" >&2
  exit 1
}

absolute_path() {
  local path="$1"
  case "$path" in
    /*) printf '%s\n' "$path" ;;
    *) printf '%s/%s\n' "$caller_dir" "$path" ;;
  esac
}

single_spec_in_dir() {
  local target_dir="$1"
  local specs=()

  while IFS= read -r -d '' path; do
    specs+=("$path")
  done < <(find "$target_dir" -maxdepth 1 -type f -name '*.spec' -print0 | sort -z)

  case "${#specs[@]}" in
    0) fail "no *.spec found in $target_dir; pass --spec or --base-tar with --no-spec-update" ;;
    1) printf '%s\n' "${specs[0]}" ;;
    *) fail "multiple *.spec files found in $target_dir; pass --spec" ;;
  esac
}

spec_value() {
  local spec_path="$1"
  local key="$2"

  awk -F ':' -v key="$key" '
    BEGIN { wanted = tolower(key) }
    {
      current = tolower($1)
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", current)
      if (current == wanted) {
        value = substr($0, index($0, ":") + 1)
        sub(/^[[:space:]]+/, "", value)
        sub(/[[:space:]]+$/, "", value)
        print value
        exit
      }
    }
  ' "$spec_path"
}

expand_source0() {
  local source0="$1"
  local package_name="$2"
  local package_version="$3"

  source0="${source0//%\{name\}/$package_name}"
  source0="${source0//%\{version\}/$package_version}"
  source0="${source0//%\{Name\}/$package_name}"
  source0="${source0//%\{Version\}/$package_version}"

  if [[ "$source0" == *"%{"* ]]; then
    fail "Source0 contains unsupported RPM macro after expansion: $source0"
  fi

  printf '%s\n' "$source0"
}

next_patch_index() {
  local target_dir="$1"
  local spec_path="$2"
  local max_index=0
  local index

  if [ -n "$spec_path" ] && [ -f "$spec_path" ]; then
    while IFS= read -r index; do
      if [ -z "$index" ]; then
        index=0
      fi
      index=$((10#$index))
      if [ "$index" -gt "$max_index" ]; then
        max_index="$index"
      fi
    done < <(sed -nE 's/^[[:space:]]*Patch([0-9]*)[[:space:]]*:.*/\1/p' "$spec_path")
  fi

  while IFS= read -r index; do
    if [ -z "$index" ]; then
      index=0
    fi
    index=$((10#$index))
    if [ "$index" -gt "$max_index" ]; then
      max_index="$index"
    fi
  done < <(
    find "$target_dir" -maxdepth 1 -type f -name '*.patch' -printf '%f\n' |
      sed -nE 's/^0*([0-9]+)[-.].*\.patch$/\1/p'
  )

  printf '%s\n' "$((max_index + 1))"
}

archive_top_dir() {
  local tar_path="$1"
  local tops=()

  while IFS= read -r top; do
    if [ -n "$top" ]; then
      tops+=("$top")
    fi
  done < <(tar -tzf "$tar_path" | awk -F '/' 'NF > 0 { print $1 }' | sort -u)

  if [ "${#tops[@]}" -ne 1 ]; then
    fail "expected exactly one top-level directory in $tar_path, found ${#tops[@]}"
  fi

  printf '%s\n' "${tops[0]}"
}

copy_working_tree() {
  local source_dir="$1"
  local new_root="$2"
  local working_tree_patch

  git -C "$source_dir" archive --format=tar HEAD | tar -xf - -C "$new_root"
  working_tree_patch="$(dirname "$new_root")/working-tree.patch"
  git -C "$source_dir" diff --binary --full-index --no-ext-diff HEAD -- >"$working_tree_patch"
  if [ ! -s "$working_tree_patch" ]; then
    return
  fi

  git -c core.autocrlf=false -c core.safecrlf=false -C "$new_root" init --quiet
  git -c core.autocrlf=false -c core.safecrlf=false -C "$new_root" add --force --all
  git -c user.name=actrail-patch -c user.email=actrail-patch@localhost \
    -c commit.gpgsign=false \
    -C "$new_root" commit --quiet --allow-empty -m head
  git -c core.autocrlf=false -c core.safecrlf=false -C "$new_root" \
    apply --index --binary -p1 "$working_tree_patch" ||
    fail "failed to apply tracked working-tree changes over HEAD"
  rm -rf "$new_root/.git"
  rm -f "$working_tree_patch"
}

preserve_source0_paths() {
  local old_root="$1"
  local new_root="$2"
  shift 2
  local rel

  for rel in "$@"; do
    rel="${rel#/}"
    if [ -z "$rel" ]; then
      fail "--preserve-path must not be empty or /"
    fi
    if [ -e "$new_root/$rel" ] || [ -L "$new_root/$rel" ]; then
      continue
    fi
    if [ ! -e "$old_root/$rel" ] && [ ! -L "$old_root/$rel" ]; then
      continue
    fi
    mkdir -p "$new_root/$(dirname "$rel")"
    cp -a "$old_root/$rel" "$new_root/$rel"
  done
}

write_patch() {
  local trees_dir="$1"
  local raw_patch="$2"
  local output_patch="$3"
  local diff_status

  set +e
  (
    cd "$trees_dir"
    git -c core.autocrlf=false -c core.safecrlf=false \
      diff --no-index --binary --full-index --no-renames --no-prefix a b
  ) > "$raw_patch"
  diff_status=$?
  set -e

  if [ "$diff_status" -gt 1 ]; then
    fail "diff failed with exit status $diff_status"
  fi
  if [ "$diff_status" -eq 0 ]; then
    fail "no differences found between Source0 and requested source tree"
  fi
  cp "$raw_patch" "$output_patch"
}

existing_patch_files() {
  local spec_path="$1"

  awk '
    /^[[:space:]]*Patch[0-9]*[[:space:]]*:/ {
      value = substr($0, index($0, ":") + 1)
      sub(/^[[:space:]]+/, "", value)
      sub(/[[:space:]]+$/, "", value)
      split(value, fields, /[[:space:]]+/)
      if (fields[1] != "") {
        print fields[1]
      }
    }
  ' "$spec_path"
}

apply_existing_patches() {
  local target_dir="$1"
  local spec_path="$2"
  local old_root="$3"
  local patch_ref patch_path
  local patch_refs=()

  mapfile -t patch_refs < <(existing_patch_files "$spec_path")
  if [ "${#patch_refs[@]}" -eq 0 ]; then
    return
  fi

  git -c core.autocrlf=false -c core.safecrlf=false -C "$old_root" init --quiet
  git -c core.autocrlf=false -c core.safecrlf=false -C "$old_root" add --force --all
  git -c user.name=actrail-patch -c user.email=actrail-patch@localhost \
    -c commit.gpgsign=false \
    -C "$old_root" commit --quiet --allow-empty -m source0

  for patch_ref in "${patch_refs[@]}"; do
    if [[ "$patch_ref" == *"%{"* ]]; then
      fail "existing Patch entry contains an unsupported RPM macro: $patch_ref"
    fi
    patch_path="$target_dir/${patch_ref##*/}"
    [ -f "$patch_path" ] || fail "existing patch file does not exist: $patch_path"
    git -c core.autocrlf=false -c core.safecrlf=false -C "$old_root" \
      apply --index --binary -p1 "$patch_path" ||
      fail "failed to apply existing patch to Source0 baseline: $patch_path"
  done

  rm -rf "$old_root/.git"
}

existing_patch_count() {
  local spec_path="$1"

  sed -nE 's/^[[:space:]]*Patch[0-9]*[[:space:]]*:.*/patch/p' "$spec_path" | wc -l
}

ensure_spec_patch_setup() {
  local input_path="$1"
  local output_path="$2"
  local existing_count="$3"
  local status

  set +e
  awk -v existing_count="$existing_count" '
    BEGIN { seen = 0 }

    /^[[:space:]]*%autosetup([[:space:]]|$)/ {
      seen = 1

      line = $0
      if (line !~ /(^|[[:space:]])-p1([^0-9]|$)/ &&
          (line ~ /(^|[[:space:]])-p([[:space:]]|$)/ || line ~ /(^|[[:space:]])-p[0-9]+([^0-9]|$)/)) {
        print "unsupported existing %autosetup patch strip level: " $0 > "/dev/stderr"
        exit 4
      }

      if (line !~ /(^|[[:space:]])-p1([^0-9]|$)/ && existing_count > 0) {
        print "spec already has Patch lines but %autosetup has no explicit -p level" > "/dev/stderr"
        exit 5
      }

      if (line !~ /(^|[[:space:]])-p1([^0-9]|$)/) {
        line = line " -p1"
      }

      if (line ~ /(^|[[:space:]])-S/ &&
          line !~ /(^|[[:space:]])-S([[:space:]]+)?git([[:space:]]|$)/) {
        print "unsupported existing %autosetup patch backend: " $0 > "/dev/stderr"
        exit 7
      }
      if (line !~ /(^|[[:space:]])-S([[:space:]]+)?git([[:space:]]|$)/) {
        line = line " -S git"
      }

      print line
      next
    }

    { print }

    END {
      if (!seen) {
        print "spec has no %autosetup line to apply Patch entries" > "/dev/stderr"
        exit 6
      }
    }
  ' "$input_path" > "$output_path"
  status=$?
  set -e

  case "$status" in
    0) ;;
    4) fail "$input_path uses %autosetup with a patch strip level other than -p1" ;;
    5) fail "$input_path has existing patches without an explicit %autosetup -p level; set it manually before appending" ;;
    6) fail "$input_path has no %autosetup line" ;;
    7) fail "$input_path uses a %autosetup patch backend other than git" ;;
    *) fail "failed to update %autosetup patch strip level in $input_path" ;;
  esac
}

insert_patch_line() {
  local spec_path="$1"
  local patch_tag="$2"
  local patch_file="$3"
  local output_path="$4"
  local insert_line

  insert_line="$(printf '%-15s %s' "${patch_tag}:" "$patch_file")"

  awk -v insert_line="$insert_line" '
    BEGIN {
      inserted = 0
      source_block = 0
      patch_block = 0
    }

    /^[[:space:]]*Patch[0-9]*[[:space:]]*:/ {
      print
      patch_block = 1
      source_block = 0
      next
    }

    {
      if (!inserted && patch_block) {
        print insert_line
        inserted = 1
        patch_block = 0
      }

      if (!inserted && source_block && $0 !~ /^[[:space:]]*Source[0-9]*[[:space:]]*:/) {
        print insert_line
        inserted = 1
        source_block = 0
      }

      print

      if ($0 ~ /^[[:space:]]*Source[0-9]*[[:space:]]*:/) {
        source_block = 1
      } else if ($0 !~ /^[[:space:]]*$/) {
        source_block = 0
      }
    }

    END {
      if (!inserted) {
        print insert_line
      }
    }
  ' "$spec_path" > "$output_path"
}

caller_dir="$PWD"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
default_source_dir="$(git -C "$script_dir/.." rev-parse --show-toplevel)"

target_dir=""
patch_name=""
source_dir="$default_source_dir"
treeish="HEAD"
source_mode="git-archive"
base_tar=""
spec_path=""
patch_index=""
update_spec=1
use_default_preserve=1
preserve_paths=()

while [ "$#" -gt 0 ]; do
  case "$1" in
    --target-dir)
      target_dir="${2:?missing value for --target-dir}"
      shift 2
      ;;
    --name)
      patch_name="${2:?missing value for --name}"
      shift 2
      ;;
    --source-dir)
      source_dir="${2:?missing value for --source-dir}"
      shift 2
      ;;
    --tree-ish)
      treeish="${2:?missing value for --tree-ish}"
      shift 2
      ;;
    --working-tree)
      source_mode="working-tree"
      shift
      ;;
    --base-tar)
      base_tar="${2:?missing value for --base-tar}"
      shift 2
      ;;
    --spec)
      spec_path="${2:?missing value for --spec}"
      shift 2
      ;;
    --patch-index)
      patch_index="${2:?missing value for --patch-index}"
      shift 2
      ;;
    --no-spec-update)
      update_spec=0
      shift
      ;;
    --preserve-path)
      preserve_paths+=("${2:?missing value for --preserve-path}")
      shift 2
      ;;
    --no-default-preserve)
      use_default_preserve=0
      shift
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

if [ -z "$target_dir" ]; then
  fail "missing required --target-dir <rpm-source-dir>"
fi
if [ -z "$patch_name" ]; then
  fail "missing required --name <patch-name>"
fi

target_dir="$(absolute_path "$target_dir")"
source_dir="$(absolute_path "$source_dir")"
if [ -n "$base_tar" ]; then
  base_tar="$(absolute_path "$base_tar")"
fi
if [ -n "$spec_path" ]; then
  spec_path="$(absolute_path "$spec_path")"
fi

[ -d "$target_dir" ] || fail "target directory does not exist: $target_dir"
[ -d "$source_dir" ] || fail "source directory does not exist: $source_dir"
git -C "$source_dir" rev-parse --show-toplevel >/dev/null

if [ -z "$spec_path" ]; then
  if [ "$update_spec" -eq 1 ] || [ -z "$base_tar" ]; then
    spec_path="$(single_spec_in_dir "$target_dir")"
  fi
fi
if [ -n "$spec_path" ]; then
  [ -f "$spec_path" ] || fail "spec file does not exist: $spec_path"
fi

if [ -z "$base_tar" ]; then
  package_name="$(spec_value "$spec_path" "Name")"
  package_version="$(spec_value "$spec_path" "Version")"
  source0="$(spec_value "$spec_path" "Source0")"
  [ -n "$package_name" ] || fail "failed to read Name from $spec_path"
  [ -n "$package_version" ] || fail "failed to read Version from $spec_path"
  [ -n "$source0" ] || fail "failed to read Source0 from $spec_path"
  source0="$(expand_source0 "$source0" "$package_name" "$package_version")"
  base_tar="$target_dir/${source0##*/}"
fi
[ -f "$base_tar" ] || fail "Source0 tarball does not exist: $base_tar"

patch_name="${patch_name%.patch}"
if [[ ! "$patch_name" =~ ^[A-Za-z0-9._+-]+$ ]]; then
  fail "patch name must contain only letters, numbers, dot, plus, underscore, or hyphen"
fi

if [ -z "$patch_index" ]; then
  patch_index="$(next_patch_index "$target_dir" "${spec_path:-}")"
fi
if [[ ! "$patch_index" =~ ^[0-9]+$ ]]; then
  fail "--patch-index must be a non-negative integer"
fi
patch_index=$((10#$patch_index))

patch_number="$(printf '%04d' "$patch_index")"
patch_tag="Patch${patch_number}"
patch_file="${patch_number}-${patch_name}.patch"
target_patch="$target_dir/$patch_file"

[ ! -e "$target_patch" ] || fail "target patch already exists: $target_patch"
if [ -n "$spec_path" ]; then
  if grep -Eq "^[[:space:]]*${patch_tag}[[:space:]]*:" "$spec_path"; then
    fail "$patch_tag already exists in $spec_path"
  fi
  if grep -Fq "$patch_file" "$spec_path"; then
    fail "$patch_file is already referenced by $spec_path"
  fi
fi

if [ "$use_default_preserve" -eq 1 ]; then
  preserve_paths=(
    "crates/apps/web/frontend/dist"
    "crates/apps/ctl/java-agent/dist"
    "${preserve_paths[@]}"
  )
fi

staging_dir="$(mktemp -d "${TMPDIR:-/tmp}/actrail-append-patch.XXXXXXXX")"
cleanup() {
  local status=$?
  rm -rf "$staging_dir"
  exit "$status"
}
trap cleanup EXIT

base_extract="$staging_dir/base"
trees_dir="$staging_dir/trees"
old_root="$trees_dir/a"
new_root="$trees_dir/b"
raw_patch="$staging_dir/raw.patch"
generated_patch="$staging_dir/$patch_file"
generated_spec="$staging_dir/$(basename "${spec_path:-spec}")"

mkdir -p "$base_extract" "$old_root" "$new_root"

top_dir="$(archive_top_dir "$base_tar")"
tar -xzf "$base_tar" -C "$base_extract"
cp -a "$base_extract/$top_dir/." "$old_root/"

if [ -n "$spec_path" ]; then
  apply_existing_patches "$target_dir" "$spec_path" "$old_root"
fi

case "$source_mode" in
  git-archive)
    git -C "$source_dir" archive --format=tar "$treeish" | tar -xf - -C "$new_root"
    ;;
  working-tree)
    copy_working_tree "$source_dir" "$new_root"
    ;;
  *)
    fail "unknown source mode: $source_mode"
    ;;
esac

preserve_source0_paths "$old_root" "$new_root" "${preserve_paths[@]}"
write_patch "$trees_dir" "$raw_patch" "$generated_patch"

if [ "$update_spec" -eq 1 ]; then
  existing_count="$(existing_patch_count "$spec_path")"
  inserted_spec="$staging_dir/inserted-$(basename "$spec_path")"
  insert_patch_line "$spec_path" "$patch_tag" "$patch_file" "$inserted_spec"
  ensure_spec_patch_setup "$inserted_spec" "$generated_spec" "$existing_count"
fi

cp "$generated_patch" "$target_patch"
if [ "$update_spec" -eq 1 ]; then
  cp "$generated_spec" "$spec_path"
fi

printf 'patch: %s\n' "$target_patch"
if [ "$update_spec" -eq 1 ]; then
  printf 'spec: %s\n' "$spec_path"
  printf 'tag: %s\n' "$patch_tag"
fi
