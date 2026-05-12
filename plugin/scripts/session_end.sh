#!/usr/bin/env bash
# SessionEnd hook — flushes remaining turns and marks the session closed.
# Detached with nohup/disown because CC kills SessionEnd hooks before async
# work completes. Stop-per-turn is the primary reporting path.
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

# Derive log path from transcript_path so it lands in the right data dir.
if command -v jq >/dev/null 2>&1; then
  TRANSCRIPT="$(echo "${INPUT}" | jq -r '.transcript_path // ""')"
  if [[ -n "${TRANSCRIPT}" ]]; then
    DATA_DIR="$(dirname "$(dirname "$(dirname "${TRANSCRIPT}")")")"
    LOG_DIR="${DATA_DIR}/ccflux"
    mkdir -p "${LOG_DIR}" 2>/dev/null || true
    LOG="${LOG_DIR}/session_end.log"
  else
    LOG="/dev/null"
  fi
else
  LOG="/dev/null"
fi

nohup "${BIN}" session-end --input "${INPUT}" >> "${LOG}" 2>&1 &
disown
exit 0
