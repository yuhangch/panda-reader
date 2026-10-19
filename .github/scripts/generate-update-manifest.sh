#!/usr/bin/env bash
# Build docs/public/updates/stable.json from release artifacts + checksums.txt.
#
# Usage:
#   generate-update-manifest.sh <version> <dist-dir> <output-json>
#
# version may include a leading "v". dist-dir must contain checksums.txt and the
# packaged installers that the in-app updater consumes.

set -euo pipefail

raw_version="${1:?version required}"
dist_dir="${2:?dist directory required}"
output="${3:?output path required}"
version="${raw_version#v}"
tag="v${version}"
base_url="https://github.com/yuhangch/panda-reader/releases/download/${tag}"
html_url="https://github.com/yuhangch/panda-reader/releases/tag/${tag}"
checksums="${dist_dir}/checksums.txt"

if [[ ! -f "${checksums}" ]]; then
  echo "missing ${checksums}" >&2
  exit 1
fi

hash_for() {
  local name="$1"
  awk -v name="${name}" '
    $2 == name || $2 == ("*" name) { print tolower($1); found=1; exit }
    END { if (!found) exit 1 }
  ' "${checksums}"
}

emit_asset() {
  local key="$1"
  local name="$2"
  local path="${dist_dir}/${name}"
  if [[ ! -f "${path}" ]]; then
    echo "skip missing asset ${name}" >&2
    return 1
  fi
  local size
  size="$(wc -c < "${path}" | tr -d ' ')"
  local sha
  sha="$(hash_for "${name}")"
  printf '    "%s": {\n' "${key}"
  printf '      "name": "%s",\n' "${name}"
  printf '      "size": %s,\n' "${size}"
  printf '      "sha256": "%s",\n' "${sha}"
  printf '      "urls": [\n'
  printf '        "%s/%s"\n' "${base_url}" "${name}"
  printf '      ]\n'
  printf '    }'
}

assets=()
if asset="$(emit_asset "windows-x86_64-setup" "panda-reader-${version}-windows-x86_64-setup.exe")"; then
  assets+=("${asset}")
fi
if asset="$(emit_asset "macos-arm64-zip" "panda-reader-${version}-macos-arm64.zip")"; then
  assets+=("${asset}")
fi
if asset="$(emit_asset "macos-x86_64-zip" "panda-reader-${version}-macos-x86_64.zip")"; then
  assets+=("${asset}")
fi
if asset="$(emit_asset "linux-x86_64-appimage" "panda-reader-${version}-linux-x86_64.AppImage")"; then
  assets+=("${asset}")
fi

if [[ ${#assets[@]} -eq 0 ]]; then
  echo "no updater assets found in ${dist_dir}" >&2
  exit 1
fi

mkdir -p "$(dirname "${output}")"
{
  printf '{\n'
  printf '  "version": "%s",\n' "${version}"
  printf '  "channel": "stable",\n'
  printf '  "html_url": "%s",\n' "${html_url}"
  printf '  "assets": {\n'
  for index in "${!assets[@]}"; do
    printf '%s' "${assets[$index]}"
    if (( index < ${#assets[@]} - 1 )); then
      printf ','
    fi
    printf '\n'
  done
  printf '  }\n'
  printf '}\n'
} > "${output}"

python3 -m json.tool "${output}" >/dev/null
echo "wrote ${output}"
