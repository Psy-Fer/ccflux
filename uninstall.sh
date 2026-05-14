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

    raw="${HOME}/.claude"$'\n'
    for dir in "${HOME}"/.claude-*/; do
        [[ -d "${dir%/}" ]] && raw="${raw}${dir%/}"$'\n'
    done
    while IFS= read -r json_file; do
        raw="${raw}$(dirname "$json_file")"$'\n'
    done < <(find "${HOME}" -maxdepth 3 -name ".claude.json" 2>/dev/null | sort)

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
    local pybin="$1" install_dir="$2"

    CCFLUX_INSTALL_DIR="$install_dir" \
    "$pybin" - <<'PYEOF'
import json, os, shutil

install_dir = os.environ['CCFLUX_INSTALL_DIR']
plugins_dir = os.path.join(install_dir, 'plugins')
installed_j = os.path.join(plugins_dir, 'installed_plugins.json')
settings_j  = os.path.join(install_dir, 'settings.json')
known_j     = os.path.join(plugins_dir, 'known_marketplaces.json')
mkt_dir     = os.path.join(plugins_dir, 'marketplaces', 'ccflux')

if os.path.isfile(installed_j):
    with open(installed_j) as f: d = json.load(f)
    if d.get('plugins', {}).pop('ccflux@ccflux', None) is not None:
        with open(installed_j, 'w') as f: json.dump(d, f, indent=2); f.write('\n')
        print('  updated  plugins/installed_plugins.json  (removed ccflux@ccflux)')

if os.path.isfile(settings_j):
    with open(settings_j) as f: d = json.load(f)
    if d.get('enabledPlugins', {}).pop('ccflux@ccflux', None) is not None:
        with open(settings_j, 'w') as f: json.dump(d, f, indent=2); f.write('\n')
        print('  updated  settings.json  (removed ccflux@ccflux)')

if os.path.isfile(known_j):
    with open(known_j) as f: d = json.load(f)
    if d.pop('ccflux', None) is not None:
        with open(known_j, 'w') as f: json.dump(d, f, indent=2); f.write('\n')
        print('  updated  plugins/known_marketplaces.json  (removed ccflux marketplace)')

if os.path.isdir(mkt_dir):
    shutil.rmtree(mkt_dir)
    print('  removed  plugins/marketplaces/ccflux/')
PYEOF
}

_dereg_node() {
    local nodebin="$1" install_dir="$2"

    CCFLUX_INSTALL_DIR="$install_dir" \
    "$nodebin" -e "
const fs=require('fs'),path=require('path'),e=process.env;
const idir=e.CCFLUX_INSTALL_DIR;
const pdir=path.join(idir,'plugins');
const ipJ=path.join(pdir,'installed_plugins.json');
const sJ=path.join(idir,'settings.json');
const kmJ=path.join(pdir,'known_marketplaces.json');
const mktDir=path.join(pdir,'marketplaces','ccflux');

function rmrf(d){try{fs.rmSync(d,{recursive:true,force:true});}catch(_){}}
function readJ(p){try{return JSON.parse(fs.readFileSync(p,'utf8'));}catch(_){return null;}}
function writeJ(p,d){fs.writeFileSync(p,JSON.stringify(d,null,2)+'\n');}

let d;
if((d=readJ(ipJ))&&d.plugins&&'ccflux@ccflux'in d.plugins){
  delete d.plugins['ccflux@ccflux'];writeJ(ipJ,d);
  process.stdout.write('  updated  plugins/installed_plugins.json  (removed ccflux@ccflux)\n');}

if((d=readJ(sJ))&&d.enabledPlugins&&'ccflux@ccflux'in d.enabledPlugins){
  delete d.enabledPlugins['ccflux@ccflux'];writeJ(sJ,d);
  process.stdout.write('  updated  settings.json  (removed ccflux@ccflux)\n');}

if((d=readJ(kmJ))&&'ccflux'in d){
  delete d['ccflux'];writeJ(kmJ,d);
  process.stdout.write('  updated  plugins/known_marketplaces.json  (removed ccflux marketplace)\n');}

try{if(fs.statSync(mktDir).isDirectory()){rmrf(mktDir);process.stdout.write('  removed  plugins/marketplaces/ccflux/\n');}}catch(_){}"
}

_dereg_powershell() {
    local psbin="$1" install_dir="$2"
    local tmp="/tmp/ccflux_uninstall_$$.ps1"

    cat > "$tmp" << 'PSEOF'
$ErrorActionPreference = 'Stop'
$idir   = $env:CCFLUX_INSTALL_DIR
$pdir   = Join-Path $idir 'plugins'
$ipJ    = Join-Path $pdir 'installed_plugins.json'
$sJ     = Join-Path $idir 'settings.json'
$kmJ    = Join-Path $pdir 'known_marketplaces.json'
$mktDir = Join-Path $pdir 'marketplaces\ccflux'

if (Test-Path $ipJ) {
    $d = Get-Content $ipJ -Raw | ConvertFrom-Json
    if ($d.plugins.PSObject.Properties['ccflux@ccflux']) {
        $d.plugins.PSObject.Properties.Remove('ccflux@ccflux')
        $d | ConvertTo-Json -Depth 10 | Set-Content $ipJ -Encoding UTF8
        Write-Host "  updated  plugins/installed_plugins.json  (removed ccflux@ccflux)"
    }
}

if (Test-Path $sJ) {
    $s = Get-Content $sJ -Raw | ConvertFrom-Json
    if ($s.PSObject.Properties['enabledPlugins'] -and $s.enabledPlugins.PSObject.Properties['ccflux@ccflux']) {
        $s.enabledPlugins.PSObject.Properties.Remove('ccflux@ccflux')
        $s | ConvertTo-Json -Depth 10 | Set-Content $sJ -Encoding UTF8
        Write-Host "  updated  settings.json  (removed ccflux@ccflux)"
    }
}

if (Test-Path $kmJ) {
    $km = Get-Content $kmJ -Raw | ConvertFrom-Json
    if ($km.PSObject.Properties['ccflux']) {
        $km.PSObject.Properties.Remove('ccflux')
        $km | ConvertTo-Json -Depth 10 | Set-Content $kmJ -Encoding UTF8
        Write-Host "  updated  plugins/known_marketplaces.json  (removed ccflux marketplace)"
    }
}

if (Test-Path $mktDir -PathType Container) {
    Remove-Item $mktDir -Recurse -Force
    Write-Host "  removed  plugins/marketplaces/ccflux/"
}
PSEOF

    CCFLUX_INSTALL_DIR="$(_win_path "$install_dir")" \
    "$psbin" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$(_win_path "$tmp")"
    rm -f "$tmp"
}

deregister_plugin() {
    local install_dir="$1"

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
        echo "           Manually remove $(dim '"ccflux@ccflux"') from installed_plugins.json,"
        echo "           settings.json, and known_marketplaces.json."
        return
    fi

    "_dereg_${tool}" "$bin" "$install_dir"
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
