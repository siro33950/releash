#!/bin/bash
set -euo pipefail
export PATH="${HOME}/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:${PATH}"
cd "$(dirname "$0")/../../.."
cargo_target_dir="${CARGO_TARGET_DIR:-target}"
profile=debug
cargo_flags=(--profile dev)
if [[ "${CONFIGURATION}" == Release ]]; then
  profile=release
  cargo_flags=(--profile release)
fi
for arch in ${ARCHS}; do
  case "$arch" in
    arm64) rust_target=aarch64-apple-darwin ;;
    x86_64) rust_target=x86_64-apple-darwin ;;
    *) exit 1 ;;
  esac
  cargo build --locked "${cargo_flags[@]}" --target "$rust_target" -p releashd --bin releashd -p releash --bin releash
done
helpers_dir="${TARGET_BUILD_DIR}/${CONTENTS_FOLDER_PATH}/Helpers"
mkdir -p "${helpers_dir}"
for helper in releashd releash; do
  binaries=()
  for arch in ${ARCHS}; do
    case "$arch" in
      arm64) rust_target=aarch64-apple-darwin ;;
      x86_64) rust_target=x86_64-apple-darwin ;;
    esac
    binaries+=("${cargo_target_dir}/${rust_target}/${profile}/${helper}")
  done
  /usr/bin/lipo -create "${binaries[@]}" -output "${helpers_dir}/${helper}"
  if [[ -n "${EXPANDED_CODE_SIGN_IDENTITY:-}" ]]; then
    /usr/bin/codesign --force --sign "${EXPANDED_CODE_SIGN_IDENTITY}" "${helpers_dir}/${helper}"
  fi
done
