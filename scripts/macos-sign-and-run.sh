#!/usr/bin/env bash
set -euo pipefail

codesign --force --sign - --entitlements "$(dirname "$0")/macos-entitlements.plist" "$1"
rust_libdir="$(rustc --print target-libdir)"
export DYLD_LIBRARY_PATH="$rust_libdir${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}"
exec "$@"
