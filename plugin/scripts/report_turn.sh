#!/usr/bin/env bash
# Stop hook — reports token usage for the completed turn.
# Endpoint and token are read by the binary from CLAUDE_PLUGIN_OPTION_* env vars
# or <data_dir>/ccflux/config.json. Never passed as CLI args.
set -euo pipefail

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

"${BIN}" report-turn --input "${INPUT}" || true
exit 0
