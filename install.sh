#!/usr/bin/env bash
# ccflux plugin installer
#
# Finds all Claude Code data directories on this machine (including aliased
# ones), lets you pick the install target, and copies the plugin files.
#
# Run from the ccflux repo root:
#   bash install.sh

set -euo pipefail

PLUGIN_NAME="ccflux"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PLUGIN_SRC="${SCRIPT_DIR}/plugin"

# ── Colour helpers ────────────────────────────────────────────────────────────

# Disable colours when not connected to a terminal
if [ -t 1 ]; then
    _bold='\033[1m'; _reset='\033[0m'
    _green='\033[32m'; _yellow='\033[33m'; _red='\033[31m'; _dim='\033[2m'
else
    _bold=''; _reset=''; _green=''; _yellow=''; _red=''; _dim=''
fi

bold()   { printf "${_bold}%s${_reset}"   "$*"; }
green()  { printf "${_green}%s${_reset}"  "$*"; }
yellow() { printf "${_yellow}%s${_reset}" "$*"; }
red()    { printf "${_red}%s${_reset}"    "$*"; }
dim()    { printf "${_dim}%s${_reset}"    "$*"; }

die() { echo "$(red "error:") $*" >&2; exit 1; }

# ── Platform detection ────────────────────────────────────────────────────────

OS="$(uname -s 2>/dev/null | tr '[:upper:]' '[:lower:]')"
is_windows_bash() {
    case "$OS" in msys*|mingw*|cygwin*) return 0 ;; *) return 1 ;; esac
}

# ── Find candidate CC data directories ───────────────────────────────────────

# A directory is a CC data dir if it contains any of:
#   .claude.json   (OAuth account config)
#   projects/      (per-project transcript storage)
#   plugins/       (installed plugins)
looks_like_claude_dir() {
    local dir="$1"
    # The home directory itself is never a CC data dir
    [[ "$dir" == "$HOME" ]] && return 1
    [[ -f "${dir}/.claude.json" || -d "${dir}/projects" || -d "${dir}/plugins" ]]
}

find_claude_dirs() {
    local dirs=()
    local seen=()

    already_seen() {
        local d="$1" s
        for s in "${seen[@]:-}"; do [[ "$s" == "$d" ]] && return 0; done
        return 1
    }

    add_if_claude() {
        local dir="${1%/}"
        [[ -d "$dir" ]] || return 0
        already_seen "$dir" && return 0
        if looks_like_claude_dir "$dir"; then
            seen+=("$dir")
            dirs+=("$dir")
        fi
    }

    # Standard default location
    add_if_claude "${HOME}/.claude"

    # Any ~/.claude-* variants (common aliased config dirs)
    for dir in "${HOME}"/.claude-*/; do
        add_if_claude "$dir"
    done

    # Broader scan: find any .claude.json up to 3 levels deep in HOME.
    # Covers CLAUDE_CONFIG_DIR pointing to non-standard paths.
    while IFS= read -r json_file; do
        add_if_claude "$(dirname "$json_file")"
    done < <(find "${HOME}" -maxdepth 3 -name ".claude.json" 2>/dev/null | sort)

    printf '%s\n' "${dirs[@]:-}"
}

# ── Preflight checks ──────────────────────────────────────────────────────────

echo ""
echo "$(bold "ccflux plugin installer")"
echo ""

[[ -d "${PLUGIN_SRC}" ]] \
    || die "plugin/ directory not found — run this script from the ccflux repo root."
[[ -d "${PLUGIN_SRC}/.claude-plugin" && -d "${PLUGIN_SRC}/scripts" && -d "${PLUGIN_SRC}/hooks" ]] \
    || die "plugin/ directory is incomplete — check your checkout."

# Warn (don't fail) if bin/ is empty — useful during development
bin_count=0
if [[ -d "${PLUGIN_SRC}/bin" ]]; then
    bin_count=$(find "${PLUGIN_SRC}/bin" -maxdepth 1 -type f | wc -l | tr -d ' ')
fi
if (( bin_count == 0 )); then
    echo "$(yellow "warning:") No binaries found in plugin/bin/."
    echo "         Download a release archive and unpack the binaries into plugin/bin/,"
    echo "         or build from source: $(dim "cd ccflux-core && cargo build --release")"
    echo ""
    printf "Continue anyway (you can add binaries after installing)? [y/N] "
    read -r ans
    [[ "$ans" =~ ^[Yy]$ ]] || exit 0
    echo ""
fi

# ── Discover install targets ──────────────────────────────────────────────────

echo "Scanning for Claude Code data directories..."
echo ""

candidates=()
while IFS= read -r dir; do
    [[ -n "$dir" ]] && candidates+=("$dir")
done < <(find_claude_dirs)

if (( ${#candidates[@]} == 0 )); then
    echo "$(yellow "No Claude Code directories found automatically.")"
    echo ""
fi

# Print found locations
if (( ${#candidates[@]} > 0 )); then
    echo "Found Claude Code installation(s):"
    echo ""
    for i in "${!candidates[@]}"; do
        dir="${candidates[$i]}"
        note=""
        [[ "$dir" == "${HOME}/.claude" ]] && note="  $(dim "(default)")"
        [[ -d "${dir}/plugins/${PLUGIN_NAME}" ]] && note="${note}  $(yellow "(already installed)")"
        printf "  $(bold "%d)") %s%s\n" "$((i+1))" "$dir" "$note"
    done
    echo "  $(bold "c)") Enter a custom path"
    echo ""
fi

# Prompt for choice
while true; do
    if (( ${#candidates[@]} > 0 )); then
        printf "Choose installation target [1]: "
    else
        printf "Enter a custom path (e.g. ~/.claude): "
    fi
    read -r choice
    choice="${choice:-1}"

    if [[ "$choice" == "c" || "$choice" == "C" || (( ${#candidates[@]} == 0 )) ]]; then
        [[ "$choice" =~ ^[cC]$|^[cC]$ || (( ${#candidates[@]} == 0 )) ]] || true
        if [[ "$choice" != "c" && "$choice" != "C" && (( ${#candidates[@]} == 0 )) ]]; then
            # The user's input IS the path when no candidates were found
            INSTALL_DIR="${choice/#\~/$HOME}"
        else
            printf "Path: "
            read -r custom_path
            INSTALL_DIR="${custom_path/#\~/$HOME}"
        fi
        [[ -n "$INSTALL_DIR" ]] || { echo "Path cannot be empty."; continue; }
        break
    elif [[ "$choice" =~ ^[0-9]+$ ]] \
         && (( choice >= 1 && choice <= ${#candidates[@]} )); then
        INSTALL_DIR="${candidates[$((choice-1))]}"
        break
    else
        echo "Invalid choice — enter a number between 1 and ${#candidates[@]}, or 'c'."
    fi
done

PLUGIN_DEST="${INSTALL_DIR}/plugins/${PLUGIN_NAME}"

echo ""
echo "Installing to: $(bold "${PLUGIN_DEST}")"

# ── Hooks variant ─────────────────────────────────────────────────────────────

HOOKS_SRC="${PLUGIN_SRC}/hooks/hooks.json"

if is_windows_bash; then
    echo ""
    echo "$(yellow "Windows environment detected (Git Bash / MSYS / Cygwin).")"
    echo "How does Claude Code run on this machine?"
    echo ""
    echo "  $(bold "1)") WSL or Git Bash  $(dim "— hooks.json  (recommended)")"
    echo "  $(bold "2)") Native PowerShell $(dim "— hooks-windows.json")"
    echo ""
    printf "Choice [1]: "
    read -r win_choice
    win_choice="${win_choice:-1}"
    if [[ "$win_choice" == "2" ]]; then
        HOOKS_SRC="${PLUGIN_SRC}/hooks/hooks-windows.json"
        echo "$(yellow "Using PowerShell hooks variant.")"
    else
        echo "Using standard hooks (Git Bash compatible)."
    fi
fi

# ── Copy files ────────────────────────────────────────────────────────────────

echo ""

# .claude-plugin/
mkdir -p "${PLUGIN_DEST}/.claude-plugin"
cp "${PLUGIN_SRC}/.claude-plugin/plugin.json" "${PLUGIN_DEST}/.claude-plugin/"
echo "  copied  .claude-plugin/plugin.json"

# hooks/ — install chosen variant as hooks.json
mkdir -p "${PLUGIN_DEST}/hooks"
cp "${HOOKS_SRC}" "${PLUGIN_DEST}/hooks/hooks.json"
echo "  copied  hooks/hooks.json"

# scripts/ — copy all, set executable bits
mkdir -p "${PLUGIN_DEST}/scripts"
for f in "${PLUGIN_SRC}/scripts/"*; do
    cp "$f" "${PLUGIN_DEST}/scripts/"
    fname="$(basename "$f")"
    [[ "$fname" == *.sh ]] && chmod +x "${PLUGIN_DEST}/scripts/${fname}"
    echo "  copied  scripts/${fname}"
done

# bin/ — copy all, set executable bits
if [[ -d "${PLUGIN_SRC}/bin" ]] && (( bin_count > 0 )); then
    mkdir -p "${PLUGIN_DEST}/bin"
    for f in "${PLUGIN_SRC}/bin/"*; do
        cp "$f" "${PLUGIN_DEST}/bin/"
        chmod +x "${PLUGIN_DEST}/bin/$(basename "$f")" 2>/dev/null || true
        echo "  copied  bin/$(basename "$f")"
    done
else
    mkdir -p "${PLUGIN_DEST}/bin"
    echo "  created bin/  $(yellow "(empty — add binaries before using)")"
fi

# ── Register plugin in CC's plugin registry ───────────────────────────────────

register_plugin() {
    local install_dir="$1"
    local plugin_dest="$2"
    local plugins_dir="${install_dir}/plugins"
    local installed_json="${plugins_dir}/installed_plugins.json"
    local settings_json="${install_dir}/settings.json"
    local now
    now="$(date -u +%Y-%m-%dT%H:%M:%S.000Z 2>/dev/null || date -u +%Y-%m-%dT%H:%M:%S.000Z)"

    local python_bin=""
    for p in python3 python; do
        command -v "$p" &>/dev/null && "$p" -c "import sys" 2>/dev/null && { python_bin="$p"; break; }
    done

    if [[ -z "$python_bin" ]]; then
        echo "  $(yellow "warning:") python not found — plugin not registered in CC registry."
        echo "           Add $(dim "\"ccflux@local\": true") to settings.json enabledPlugins manually."
        return
    fi

    # Update installed_plugins.json
    CCFLUX_INSTALLED_JSON="$installed_json" \
    CCFLUX_PLUGIN_DEST="$plugin_dest" \
    CCFLUX_TIMESTAMP="$now" \
    "$python_bin" - <<'PYEOF'
import json, os, sys
path         = os.environ['CCFLUX_INSTALLED_JSON']
install_path = os.environ['CCFLUX_PLUGIN_DEST']
ts           = os.environ['CCFLUX_TIMESTAMP']

if os.path.isfile(path):
    with open(path) as f:
        data = json.load(f)
else:
    data = {"version": 2, "plugins": {}}

data.setdefault("version", 2)
data.setdefault("plugins", {})
data["plugins"]["ccflux@local"] = [{
    "scope":       "user",
    "installPath": install_path,
    "version":     "0.1.0",
    "installedAt": ts,
    "lastUpdated": ts,
}]

with open(path, "w") as f:
    json.dump(data, f, indent=2)
    f.write("\n")
PYEOF
    echo "  updated  plugins/installed_plugins.json  (ccflux@local)"

    # Update settings.json enabledPlugins
    CCFLUX_SETTINGS_JSON="$settings_json" \
    "$python_bin" - <<'PYEOF'
import json, os

path = os.environ['CCFLUX_SETTINGS_JSON']

if os.path.isfile(path):
    with open(path) as f:
        data = json.load(f)
else:
    data = {}

data.setdefault("enabledPlugins", {})
data["enabledPlugins"]["ccflux@local"] = True

with open(path, "w") as f:
    json.dump(data, f, indent=2)
    f.write("\n")
PYEOF
    echo "  updated  settings.json  (enabledPlugins: ccflux@local)"
}

echo ""
register_plugin "$INSTALL_DIR" "$PLUGIN_DEST"

# ── Done ──────────────────────────────────────────────────────────────────────

echo ""
echo "$(green "Done!") Plugin installed to:"
echo "       ${PLUGIN_DEST}"
echo ""
echo "$(bold "Next steps:")"
echo ""
echo "  1. Restart Claude Code (or run $(dim "/plugins refresh") if supported)."
echo ""
echo "  2. Open Claude Code settings → Plugins → ccflux and set:"
echo "       $(bold "Receiver endpoint")  your organisation's ccflux receiver URL"
echo "       $(bold "API token")          your personal token, provided by IT"
echo ""
echo "     Alternatively, create $(dim "<CC data dir>/ccflux/config.json") with:"
echo "       $(dim '{ "endpoint": "https://ccflux.example.org", "token": "rtok_..." }')"
echo ""
echo "  3. Start a session — the first turn will register your device key"
echo "     and begin reporting usage."
echo ""
if (( bin_count == 0 )); then
    echo "$(yellow "Remember:") drop the ccflux binaries into:"
    echo "  ${PLUGIN_DEST}/bin/"
    echo ""
fi
