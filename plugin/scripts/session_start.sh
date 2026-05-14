#!/usr/bin/env bash
# SessionStart hook — initialises the offset sidecar file for this session.
set -euo pipefail
INPUT="$(cat)"

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"
case "${OS}-${ARCH}" in
  linux-x86_64)   BIN="${CLAUDE_PLUGIN_ROOT}/bin/ccflux-linux-x86_64" ;;
  linux-aarch64)  BIN="${CLAUDE_PLUGIN_ROOT}/bin/ccflux-linux-aarch64" ;;
  darwin-x86_64)  BIN="${CLAUDE_PLUGIN_ROOT}/bin/ccflux-macos-x86_64" ;;
  darwin-arm64)   BIN="${CLAUDE_PLUGIN_ROOT}/bin/ccflux-macos-aarch64" ;;
  msys*|mingw*|cygwin*) BIN="${CLAUDE_PLUGIN_ROOT}/bin/ccflux-windows-x86_64.exe" ;;
  *) exit 0 ;;
esac

"${BIN}" init --input "${INPUT}" || true
exit 0
