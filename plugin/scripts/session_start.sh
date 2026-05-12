#!/usr/bin/env bash
# SessionStart hook — initialises the offset sidecar file for this session.
set -euo pipefail
# Allow plain HTTP for local dev. Remove before production deployment.
export CCFLUX_ALLOW_HTTP=1

INPUT="$(cat)"

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"
case "${OS}-${ARCH}" in
  linux-x86_64)   BIN="${CLAUDE_PLUGIN_ROOT}/bin/ccflux-linux-x86_64" ;;
  linux-aarch64)  BIN="${CLAUDE_PLUGIN_ROOT}/bin/ccflux-linux-aarch64" ;;
  darwin-x86_64)  BIN="${CLAUDE_PLUGIN_ROOT}/bin/ccflux-macos-x86_64" ;;
  darwin-arm64)   BIN="${CLAUDE_PLUGIN_ROOT}/bin/ccflux-macos-aarch64" ;;
  *) exit 0 ;;
esac

"${BIN}" init --input "${INPUT}" || true
exit 0
