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

looks_like_claude_dir() {
    local dir="$1"
    [[ "$dir" == "$HOME" ]] && return 1
    [[ -f "${dir}/.claude.json" || -d "${dir}/projects" || -d "${dir}/plugins" ]]
}

find_claude_dirs() {
    local raw="" dir json_file seen=""

    # Collect raw candidates
    raw="${HOME}/.claude"$'\n'
    for dir in "${HOME}"/.claude-*/; do
        [[ -d "${dir%/}" ]] && raw="${raw}${dir%/}"$'\n'
    done
    while IFS= read -r json_file; do
        raw="${raw}$(dirname "$json_file")"$'\n'
    done < <(find "${HOME}" -maxdepth 3 -name ".claude.json" 2>/dev/null | sort)

    # Deduplicate and filter to valid CC data dirs
    while IFS= read -r dir; do
        [[ -z "$dir" ]] && continue
        case "|$seen|" in *"|${dir}|"*) continue ;; esac
        seen="${seen}${dir}|"
        looks_like_claude_dir "$dir" || continue
        printf '%s\n' "$dir"
    done <<< "$raw"
}

# ── JSON tool helpers ─────────────────────────────────────────────────────────

# Convert a Unix/MSYS path to a Windows path for PowerShell (Git Bash only)
_win_path() {
    if command -v cygpath &>/dev/null; then cygpath -w "$1"; else printf '%s' "$1"; fi
}

_reg_python() {
    local pybin="$1" installed_json="$2" settings_json="$3" plugin_dest="$4" now="$5"

    CCFLUX_INSTALLED_JSON="$installed_json" \
    CCFLUX_PLUGIN_DEST="$plugin_dest" \
    CCFLUX_TIMESTAMP="$now" \
    "$pybin" - <<'PYEOF'
import json, os
path, dest, ts = os.environ['CCFLUX_INSTALLED_JSON'], os.environ['CCFLUX_PLUGIN_DEST'], os.environ['CCFLUX_TIMESTAMP']
d = {"version": 2, "plugins": {}}
try:
    with open(path) as f: d = json.load(f)
except OSError: pass
d.setdefault("plugins", {})
d["plugins"]["ccflux@local"] = [{"scope":"user","installPath":dest,"version":"0.1.0","installedAt":ts,"lastUpdated":ts}]
with open(path, "w") as f: json.dump(d, f, indent=2); f.write("\n")
PYEOF

    CCFLUX_SETTINGS_JSON="$settings_json" \
    "$pybin" - <<'PYEOF'
import json, os
path = os.environ['CCFLUX_SETTINGS_JSON']
d = {}
try:
    with open(path) as f: d = json.load(f)
except OSError: pass
d.setdefault("enabledPlugins", {})
d["enabledPlugins"]["ccflux@local"] = True
with open(path, "w") as f: json.dump(d, f, indent=2); f.write("\n")
PYEOF
}

_reg_node() {
    local nodebin="$1" installed_json="$2" settings_json="$3" plugin_dest="$4" now="$5"

    CCFLUX_INSTALLED_JSON="$installed_json" \
    CCFLUX_PLUGIN_DEST="$plugin_dest" \
    CCFLUX_TIMESTAMP="$now" \
    "$nodebin" -e "
const fs=require('fs'),e=process.env,p=e.CCFLUX_INSTALLED_JSON,dest=e.CCFLUX_PLUGIN_DEST,ts=e.CCFLUX_TIMESTAMP;
let d={version:2,plugins:{}};try{d=JSON.parse(fs.readFileSync(p,'utf8'));}catch(_){}
if(!d.plugins)d.plugins={};
d.plugins['ccflux@local']=[{scope:'user',installPath:dest,version:'0.1.0',installedAt:ts,lastUpdated:ts}];
fs.writeFileSync(p,JSON.stringify(d,null,2)+'\n');"

    CCFLUX_SETTINGS_JSON="$settings_json" \
    "$nodebin" -e "
const fs=require('fs'),e=process.env,p=e.CCFLUX_SETTINGS_JSON;
let d={};try{d=JSON.parse(fs.readFileSync(p,'utf8'));}catch(_){}
if(!d.enabledPlugins)d.enabledPlugins={};
d.enabledPlugins['ccflux@local']=true;
fs.writeFileSync(p,JSON.stringify(d,null,2)+'\n');"
}

_reg_powershell() {
    local psbin="$1" installed_json="$2" settings_json="$3" plugin_dest="$4" now="$5"
    local tmp="/tmp/ccflux_install_$$.ps1"

    cat > "$tmp" << 'PSEOF'
$ErrorActionPreference = 'Stop'
$ipath = $env:CCFLUX_INSTALLED_JSON
$spath = $env:CCFLUX_SETTINGS_JSON
$dest  = $env:CCFLUX_PLUGIN_DEST
$ts    = $env:CCFLUX_TIMESTAMP

if (Test-Path $ipath) { $d = Get-Content $ipath -Raw | ConvertFrom-Json }
else { $d = [PSCustomObject]@{ version = 2; plugins = [PSCustomObject]@{} } }
if (-not $d.PSObject.Properties['plugins']) {
    $d | Add-Member -NotePropertyName 'plugins' -NotePropertyValue ([PSCustomObject]@{}) -Force
}
$entry = @([PSCustomObject]@{ scope='user'; installPath=$dest; version='0.1.0'; installedAt=$ts; lastUpdated=$ts })
$d.plugins | Add-Member -NotePropertyName 'ccflux@local' -NotePropertyValue $entry -Force
$d | ConvertTo-Json -Depth 10 | Set-Content $ipath -Encoding UTF8

if (Test-Path $spath) { $s = Get-Content $spath -Raw | ConvertFrom-Json }
else { $s = [PSCustomObject]@{} }
if (-not $s.PSObject.Properties['enabledPlugins']) {
    $s | Add-Member -NotePropertyName 'enabledPlugins' -NotePropertyValue ([PSCustomObject]@{}) -Force
}
$s.enabledPlugins | Add-Member -NotePropertyName 'ccflux@local' -NotePropertyValue $true -Force
$s | ConvertTo-Json -Depth 10 | Set-Content $spath -Encoding UTF8
PSEOF

    CCFLUX_INSTALLED_JSON="$(_win_path "$installed_json")" \
    CCFLUX_SETTINGS_JSON="$(_win_path "$settings_json")" \
    CCFLUX_PLUGIN_DEST="$(_win_path "$plugin_dest")" \
    CCFLUX_TIMESTAMP="$now" \
    "$psbin" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$(_win_path "$tmp")"
    rm -f "$tmp"
}

register_plugin() {
    local install_dir="$1" plugin_dest="$2"
    local installed_json="${install_dir}/plugins/installed_plugins.json"
    local settings_json="${install_dir}/settings.json"
    local now
    now="$(date -u +%Y-%m-%dT%H:%M:%S.000Z)"

    local tool="" bin=""

    for p in python3 python; do
        command -v "$p" &>/dev/null && "$p" -c "import sys" 2>/dev/null \
            && tool="python" bin="$p" && break
    done

    if [[ -z "$tool" ]]; then
        for p in node nodejs; do
            command -v "$p" &>/dev/null && "$p" -e "process.exit(0)" 2>/dev/null \
                && tool="node" bin="$p" && break
        done
    fi

    if [[ -z "$tool" ]]; then
        for p in powershell.exe pwsh; do
            command -v "$p" &>/dev/null && tool="powershell" bin="$p" && break
        done
    fi

    if [[ -z "$tool" ]]; then
        echo "  $(yellow "warning:") No JSON tool found (python/node/powershell)."
        echo "           Manually add $(dim '"ccflux@local": true') to ${settings_json} enabledPlugins."
        return
    fi

    "_reg_${tool}" "$bin" "$installed_json" "$settings_json" "$plugin_dest" "$now"
    echo "  updated  plugins/installed_plugins.json  (ccflux@local)"
    echo "  updated  settings.json  (enabledPlugins: ccflux@local)"
}

# ── Preflight checks ──────────────────────────────────────────────────────────

echo ""
echo "$(bold "ccflux plugin installer")"
echo ""

[[ -d "${PLUGIN_SRC}" ]] \
    || die "plugin/ directory not found — run this script from the ccflux repo root."
[[ -d "${PLUGIN_SRC}/.claude-plugin" && -d "${PLUGIN_SRC}/scripts" && -d "${PLUGIN_SRC}/hooks" ]] \
    || die "plugin/ directory is incomplete — check your checkout."

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
        if [[ "$choice" != "c" && "$choice" != "C" && (( ${#candidates[@]} == 0 )) ]]; then
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

mkdir -p "${PLUGIN_DEST}/.claude-plugin"
cp "${PLUGIN_SRC}/.claude-plugin/plugin.json" "${PLUGIN_DEST}/.claude-plugin/"
echo "  copied  .claude-plugin/plugin.json"

mkdir -p "${PLUGIN_DEST}/hooks"
cp "${HOOKS_SRC}" "${PLUGIN_DEST}/hooks/hooks.json"
echo "  copied  hooks/hooks.json"

mkdir -p "${PLUGIN_DEST}/scripts"
for f in "${PLUGIN_SRC}/scripts/"*; do
    cp "$f" "${PLUGIN_DEST}/scripts/"
    fname="$(basename "$f")"
    [[ "$fname" == *.sh ]] && chmod +x "${PLUGIN_DEST}/scripts/${fname}"
    echo "  copied  scripts/${fname}"
done

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
