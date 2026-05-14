#!/usr/bin/env bash
# ccflux plugin uninstaller
#
# Finds all Claude Code data directories where ccflux is installed,
# lets you pick the target, removes plugin files, and deregisters
# from CC's plugin registry.
#
# Run from the ccflux repo root (or anywhere):
#   bash uninstall.sh

set -euo pipefail

PLUGIN_NAME="ccflux"

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
dim()    { printf "${_dim}%s${_reset}"    "$*"; }

# ── Find installed locations ──────────────────────────────────────────────────

looks_like_claude_dir() {
    local dir="$1"
    [[ "$dir" == "$HOME" ]] && return 1
    [[ -f "${dir}/.claude.json" || -d "${dir}/projects" || -d "${dir}/plugins" ]]
}

find_installed_dirs() {
    local raw="" dir json_file seen=""

    # Collect raw candidates
    raw="${HOME}/.claude"$'\n'
    for dir in "${HOME}"/.claude-*/; do
        [[ -d "${dir%/}" ]] && raw="${raw}${dir%/}"$'\n'
    done
    while IFS= read -r json_file; do
        raw="${raw}$(dirname "$json_file")"$'\n'
    done < <(find "${HOME}" -maxdepth 3 -name ".claude.json" 2>/dev/null | sort)

    # Deduplicate, filter to dirs where ccflux is actually installed
    while IFS= read -r dir; do
        [[ -z "$dir" ]] && continue
        case "|$seen|" in *"|${dir}|"*) continue ;; esac
        seen="${seen}${dir}|"
        [[ -d "$dir" ]] || continue
        looks_like_claude_dir "$dir" || continue
        [[ -d "${dir}/plugins/${PLUGIN_NAME}" ]] || continue
        printf '%s\n' "$dir"
    done <<< "$raw"
}

# ── JSON tool helpers ─────────────────────────────────────────────────────────

_win_path() {
    if command -v cygpath &>/dev/null; then cygpath -w "$1"; else printf '%s' "$1"; fi
}

_dereg_python() {
    local pybin="$1" installed_json="$2" settings_json="$3"

    if [[ -f "$installed_json" ]]; then
        CCFLUX_INSTALLED_JSON="$installed_json" \
        "$pybin" - <<'PYEOF'
import json, os
path = os.environ['CCFLUX_INSTALLED_JSON']
with open(path) as f: d = json.load(f)
if d.get("plugins", {}).pop("ccflux@local", None) is not None:
    with open(path, "w") as f: json.dump(d, f, indent=2); f.write("\n")
    print("  updated  plugins/installed_plugins.json  (removed ccflux@local)")
PYEOF
    fi

    if [[ -f "$settings_json" ]]; then
        CCFLUX_SETTINGS_JSON="$settings_json" \
        "$pybin" - <<'PYEOF'
import json, os
path = os.environ['CCFLUX_SETTINGS_JSON']
with open(path) as f: d = json.load(f)
if d.get("enabledPlugins", {}).pop("ccflux@local", None) is not None:
    with open(path, "w") as f: json.dump(d, f, indent=2); f.write("\n")
    print("  updated  settings.json  (removed ccflux@local)")
PYEOF
    fi
}

_dereg_node() {
    local nodebin="$1" installed_json="$2" settings_json="$3"

    if [[ -f "$installed_json" ]]; then
        CCFLUX_INSTALLED_JSON="$installed_json" \
        "$nodebin" -e "
const fs=require('fs'),p=process.env.CCFLUX_INSTALLED_JSON;
try{
  let d=JSON.parse(fs.readFileSync(p,'utf8'));
  if(d.plugins&&'ccflux@local'in d.plugins){
    delete d.plugins['ccflux@local'];
    fs.writeFileSync(p,JSON.stringify(d,null,2)+'\n');
    process.stdout.write('  updated  plugins/installed_plugins.json  (removed ccflux@local)\n');
  }
}catch(_){}"
    fi

    if [[ -f "$settings_json" ]]; then
        CCFLUX_SETTINGS_JSON="$settings_json" \
        "$nodebin" -e "
const fs=require('fs'),p=process.env.CCFLUX_SETTINGS_JSON;
try{
  let d=JSON.parse(fs.readFileSync(p,'utf8'));
  if(d.enabledPlugins&&'ccflux@local'in d.enabledPlugins){
    delete d.enabledPlugins['ccflux@local'];
    fs.writeFileSync(p,JSON.stringify(d,null,2)+'\n');
    process.stdout.write('  updated  settings.json  (removed ccflux@local)\n');
  }
}catch(_){}"
    fi
}

_dereg_powershell() {
    local psbin="$1" installed_json="$2" settings_json="$3"
    local tmp="/tmp/ccflux_uninstall_$$.ps1"

    cat > "$tmp" << 'PSEOF'
$ErrorActionPreference = 'Stop'
$ipath = $env:CCFLUX_INSTALLED_JSON
$spath = $env:CCFLUX_SETTINGS_JSON

if ($ipath -and (Test-Path $ipath)) {
    $d = Get-Content $ipath -Raw | ConvertFrom-Json
    if ($d.plugins.PSObject.Properties['ccflux@local']) {
        $d.plugins.PSObject.Properties.Remove('ccflux@local')
        $d | ConvertTo-Json -Depth 10 | Set-Content $ipath -Encoding UTF8
        Write-Host "  updated  plugins/installed_plugins.json  (removed ccflux@local)"
    }
}

if ($spath -and (Test-Path $spath)) {
    $s = Get-Content $spath -Raw | ConvertFrom-Json
    if ($s.PSObject.Properties['enabledPlugins'] -and $s.enabledPlugins.PSObject.Properties['ccflux@local']) {
        $s.enabledPlugins.PSObject.Properties.Remove('ccflux@local')
        $s | ConvertTo-Json -Depth 10 | Set-Content $spath -Encoding UTF8
        Write-Host "  updated  settings.json  (removed ccflux@local)"
    }
}
PSEOF

    CCFLUX_INSTALLED_JSON="$(_win_path "$installed_json")" \
    CCFLUX_SETTINGS_JSON="$(_win_path "$settings_json")" \
    "$psbin" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$(_win_path "$tmp")"
    rm -f "$tmp"
}

deregister_plugin() {
    local install_dir="$1"
    local installed_json="${install_dir}/plugins/installed_plugins.json"
    local settings_json="${install_dir}/settings.json"

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
        echo "  $(yellow "warning:") No JSON tool found — registry not updated."
        echo "           Manually remove $(dim '"ccflux@local"') from installed_plugins.json and settings.json."
        return
    fi

    "_dereg_${tool}" "$bin" "$installed_json" "$settings_json"
}

# ── Main ──────────────────────────────────────────────────────────────────────

echo ""
echo "$(bold "ccflux plugin uninstaller")"
echo ""

candidates=()
while IFS= read -r dir; do
    [[ -n "$dir" ]] && candidates+=("$dir")
done < <(find_installed_dirs)

if (( ${#candidates[@]} == 0 )); then
    echo "No ccflux installations found."
    echo ""
    exit 0
fi

echo "Found ccflux installation(s):"
echo ""
for i in "${!candidates[@]}"; do
    dir="${candidates[$i]}"
    note=""
    [[ "$dir" == "${HOME}/.claude" ]] && note="  $(dim "(default)")"
    printf "  $(bold "%d)") %s%s\n" "$((i+1))" "$dir" "$note"
done
if (( ${#candidates[@]} > 1 )); then
    echo "  $(bold "a)") All of the above"
fi
echo ""

to_remove=()
while true; do
    printf "Choose installation to remove [1]: "
    read -r choice
    choice="${choice:-1}"

    if [[ "$choice" == "a" || "$choice" == "A" ]] && (( ${#candidates[@]} > 1 )); then
        to_remove=("${candidates[@]}")
        break
    elif [[ "$choice" =~ ^[0-9]+$ ]] \
         && (( choice >= 1 && choice <= ${#candidates[@]} )); then
        to_remove=("${candidates[$((choice-1))]}")
        break
    else
        if (( ${#candidates[@]} > 1 )); then
            echo "Invalid choice — enter a number between 1 and ${#candidates[@]}, or 'a'."
        else
            echo "Invalid choice — enter 1."
        fi
    fi
done

echo ""
printf "Also remove ccflux data (signing key, token cache, pending queue)? [y/N] "
read -r remove_data
remove_data="${remove_data:-N}"

echo ""

for install_dir in "${to_remove[@]}"; do
    plugin_dir="${install_dir}/plugins/${PLUGIN_NAME}"
    data_dir="${install_dir}/${PLUGIN_NAME}"

    echo "Removing from: $(bold "${install_dir}")"

    rm -rf "$plugin_dir"
    echo "  removed  plugins/${PLUGIN_NAME}/"

    deregister_plugin "$install_dir"

    if [[ "$remove_data" =~ ^[Yy]$ ]] && [[ -d "$data_dir" ]]; then
        rm -rf "$data_dir"
        echo "  removed  ${PLUGIN_NAME}/  (data)"
    elif [[ -d "$data_dir" ]]; then
        echo "  kept     ${PLUGIN_NAME}/  $(dim "(data preserved — remove manually if desired)")"
    fi

    echo ""
done

echo "$(green "Done!") Restart Claude Code for the change to take effect."
echo ""
