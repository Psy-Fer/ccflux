#!/usr/bin/env bash
# SessionStart hook — initialises the offset sidecar file for this session.
set -euo pipefail
INPUT="$(cat)"

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"
case "${OS}-${ARCH}" in
  linux-x86_64)         BIN_NAME="ccflux-linux-x86_64" ;;
  linux-aarch64)        BIN_NAME="ccflux-linux-aarch64" ;;
  darwin-x86_64)        BIN_NAME="ccflux-macos-x86_64" ;;
  darwin-arm64)         BIN_NAME="ccflux-macos-aarch64" ;;
  msys*|mingw*|cygwin*) BIN_NAME="ccflux-windows-x86_64.exe" ;;
  *) exit 0 ;;
esac

BIN="${CLAUDE_PLUGIN_ROOT}/bin/${BIN_NAME}"

if [[ ! -f "$BIN" || ! -x "$BIN" ]] && [[ ! -f "${CLAUDE_PLUGIN_ROOT}/bin/.no-auto-download" ]]; then
    (
        plugin_json="${CLAUDE_PLUGIN_ROOT}/.claude-plugin/plugin.json"
        [[ -f "$plugin_json" ]] || exit 0
        ver="$(sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$plugin_json" | head -1)"
        [[ -n "$ver" ]] || exit 0
        url="https://github.com/psy-fer/ccflux/releases/download/v${ver}/${BIN_NAME}"
        mkdir -p "${CLAUDE_PLUGIN_ROOT}/bin"
        if command -v curl &>/dev/null; then
            curl -fsSL -o "${BIN}" "${url}" || rm -f "${BIN}"
        elif command -v wget &>/dev/null; then
            wget -qO "${BIN}" "${url}" || rm -f "${BIN}"
        fi
        [[ -f "${BIN}" ]] && chmod +x "${BIN}"
    ) || true
fi

[[ -f "$BIN" && -x "$BIN" ]] || exit 0

"${BIN}" init --input "${INPUT}" || true
exit 0
