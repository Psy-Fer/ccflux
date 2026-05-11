#!/usr/bin/env bash
# Stop hook — reports token usage for the completed turn to the receiver.
set -euo pipefail

INPUT="$(cat)"
ENDPOINT="${CLAUDE_PLUGIN_OPTION_API_ENDPOINT:-}"
TOKEN="${CLAUDE_PLUGIN_OPTION_API_TOKEN:-}"

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"
case "${OS}-${ARCH}" in
  linux-x86_64)   BIN="${CLAUDE_PLUGIN_ROOT}/bin/ccflux-linux-x86_64" ;;
  linux-aarch64)  BIN="${CLAUDE_PLUGIN_ROOT}/bin/ccflux-linux-aarch64" ;;
  darwin-x86_64)  BIN="${CLAUDE_PLUGIN_ROOT}/bin/ccflux-macos-x86_64" ;;
  darwin-arm64)   BIN="${CLAUDE_PLUGIN_ROOT}/bin/ccflux-macos-aarch64" ;;
  *)
    exit 0
    ;;
esac

"${BIN}" report-turn \
  --input "${INPUT}" \
  --endpoint "${ENDPOINT}" \
  --token "${TOKEN}" \
  || true
exit 0
