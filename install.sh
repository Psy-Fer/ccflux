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

# ── Flags ─────────────────────────────────────────────────────────────────────
OFFLINE=false
while [[ $# -gt 0 ]]; do
    case "$1" in
        --offline) OFFLINE=true; shift ;;
        *) echo "error: unknown option: $1" >&2; exit 1 ;;
    esac
done

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
        looks_like_claude_dir "$dir" || continue
        printf '%s\n' "$dir"
    done <<< "$raw"
}

# ── JSON tool helpers ─────────────────────────────────────────────────────────

_win_path() {
    if command -v cygpath &>/dev/null; then cygpath -w "$1"; else printf '%s' "$1"; fi
}

_reg_python() {
    local pybin="$1" install_dir="$2" plugin_dest="$3" now="$4"

    CCFLUX_INSTALL_DIR="$install_dir" \
    CCFLUX_PLUGIN_DEST="$plugin_dest" \
    CCFLUX_TIMESTAMP="$now" \
    CCFLUX_OFFLINE="${OFFLINE}" \
    "$pybin" - <<'PYEOF'
import json, os

install_dir  = os.environ['CCFLUX_INSTALL_DIR']
plugin_dest  = os.environ['CCFLUX_PLUGIN_DEST']
ts           = os.environ['CCFLUX_TIMESTAMP']
offline      = os.environ.get('CCFLUX_OFFLINE') == 'true'

plugins_dir  = os.path.join(install_dir, 'plugins')
installed_j  = os.path.join(plugins_dir, 'installed_plugins.json')
settings_j   = os.path.join(install_dir, 'settings.json')
known_j      = os.path.join(plugins_dir, 'known_marketplaces.json')
mkt_dir      = os.path.join(plugins_dir, 'marketplaces', 'ccflux')

if offline:
    os.makedirs(os.path.join(mkt_dir, '.claude-plugin'), exist_ok=True)
    os.makedirs(os.path.join(mkt_dir, 'plugins', 'ccflux'), exist_ok=True)
    catalog_offline = {
        "$schema": "https://anthropic.com/claude-code/marketplace.schema.json",
        "name": "ccflux",
        "description": "ccflux — per-turn token usage telemetry for Claude Code",
        "owner": {"name": "Psy-Fer", "email": "j.ferguson@garvan.org.au"},
        "plugins": [{
            "name": "ccflux",
            "description": "Per-turn token usage telemetry for Claude Code. Ships usage metadata to your organisation's self-hosted receiver.",
            "author": {"name": "Psy-Fer"},
            "category": "monitoring",
            "source": {"source": "directory", "path": plugin_dest},
            "homepage": "https://github.com/psy-fer/ccflux"
        }]
    }
    with open(os.path.join(mkt_dir, '.claude-plugin', 'marketplace.json'), 'w') as f:
        json.dump(catalog_offline, f, indent=2); f.write('\n')
    km = {}
    try:
        with open(known_j) as f: km = json.load(f)
    except OSError: pass
    km['ccflux'] = {'source': {'source': 'directory', 'path': mkt_dir}, 'installLocation': mkt_dir, 'lastUpdated': ts}
    with open(known_j, 'w') as f: json.dump(km, f, indent=2); f.write('\n')
else:
    os.makedirs(os.path.join(mkt_dir, '.claude-plugin'), exist_ok=True)
    os.makedirs(os.path.join(mkt_dir, 'plugins', 'ccflux'), exist_ok=True)
    catalog = {
        "$schema": "https://anthropic.com/claude-code/marketplace.schema.json",
        "name": "ccflux",
        "description": "ccflux — per-turn token usage telemetry for Claude Code",
        "owner": {"name": "Psy-Fer", "email": "j.ferguson@garvan.org.au"},
        "plugins": [{
            "name": "ccflux",
            "description": "Per-turn token usage telemetry for Claude Code. Ships usage metadata to your organisation's self-hosted receiver.",
            "author": {"name": "Psy-Fer"},
            "category": "monitoring",
            "source": {"source": "git-subdir", "url": "https://github.com/psy-fer/ccflux.git", "path": "plugin", "ref": "v0.1.0"},
            "homepage": "https://github.com/psy-fer/ccflux"
        }]
    }
    with open(os.path.join(mkt_dir, '.claude-plugin', 'marketplace.json'), 'w') as f:
        json.dump(catalog, f, indent=2); f.write('\n')
    km = {}
    try:
        with open(known_j) as f: km = json.load(f)
    except OSError: pass
    km['ccflux'] = {'source': {'source': 'github', 'repo': 'psy-fer/ccflux'}, 'installLocation': mkt_dir, 'lastUpdated': ts}
    with open(known_j, 'w') as f: json.dump(km, f, indent=2); f.write('\n')

# installed_plugins.json
ip = {'version': 2, 'plugins': {}}
try:
    with open(installed_j) as f: ip = json.load(f)
except OSError: pass
ip.setdefault('plugins', {})
ip['plugins']['ccflux@ccflux'] = [{'scope':'user','installPath':plugin_dest,'version':'0.1.0','installedAt':ts,'lastUpdated':ts}]
with open(installed_j, 'w') as f: json.dump(ip, f, indent=2); f.write('\n')

# settings.json
s = {}
try:
    with open(settings_j) as f: s = json.load(f)
except OSError: pass
s.setdefault('enabledPlugins', {})
s['enabledPlugins']['ccflux@ccflux'] = True
if offline:
    s.setdefault('extraKnownMarketplaces', {})
    s['extraKnownMarketplaces']['ccflux'] = {'source': {'source': 'directory', 'path': mkt_dir}}
with open(settings_j, 'w') as f: json.dump(s, f, indent=2); f.write('\n')
PYEOF
}

_reg_node() {
    local nodebin="$1" install_dir="$2" plugin_dest="$3" now="$4"

    CCFLUX_INSTALL_DIR="$install_dir" \
    CCFLUX_PLUGIN_DEST="$plugin_dest" \
    CCFLUX_TIMESTAMP="$now" \
    CCFLUX_OFFLINE="${OFFLINE}" \
    "$nodebin" -e "
const fs=require('fs'),path=require('path'),e=process.env;
const idir=e.CCFLUX_INSTALL_DIR,dest=e.CCFLUX_PLUGIN_DEST,ts=e.CCFLUX_TIMESTAMP,offline=e.CCFLUX_OFFLINE==='true';
const pdir=path.join(idir,'plugins');
const mktDir=path.join(pdir,'marketplaces','ccflux');
const mktCp=path.join(mktDir,'.claude-plugin');
const knownJ=path.join(pdir,'known_marketplaces.json');
function rmrf(d){try{fs.rmSync(d,{recursive:true,force:true});}catch(_){}}
function readJ(p){try{return JSON.parse(fs.readFileSync(p,'utf8'));}catch(_){return null;}}
function writeJ(p,d){fs.writeFileSync(p,JSON.stringify(d,null,2)+'\n');}

if(offline){
  [mktCp,path.join(mktDir,'plugins','ccflux')].forEach(d=>{try{fs.mkdirSync(d,{recursive:true});}catch(_){}});
  const offcat={'\$schema':'https://anthropic.com/claude-code/marketplace.schema.json',name:'ccflux',description:'ccflux — per-turn token usage telemetry for Claude Code',owner:{name:'Psy-Fer',email:'j.ferguson@garvan.org.au'},plugins:[{name:'ccflux',description:'Per-turn token usage telemetry for Claude Code. Ships usage metadata to your organisation\\'s self-hosted receiver.',author:{name:'Psy-Fer'},category:'monitoring',source:{source:'directory',path:dest},homepage:'https://github.com/psy-fer/ccflux'}]};
  writeJ(path.join(mktCp,'marketplace.json'),offcat);
  const km=readJ(knownJ)||{};
  km['ccflux']={source:{source:'directory',path:mktDir},installLocation:mktDir,lastUpdated:ts};
  writeJ(knownJ,km);
}else{
  [mktCp,path.join(mktDir,'plugins','ccflux')].forEach(d=>{try{fs.mkdirSync(d,{recursive:true});}catch(_){}});
  const catalog={'\$schema':'https://anthropic.com/claude-code/marketplace.schema.json',name:'ccflux',description:'ccflux — per-turn token usage telemetry for Claude Code',owner:{name:'Psy-Fer',email:'j.ferguson@garvan.org.au'},plugins:[{name:'ccflux',description:'Per-turn token usage telemetry for Claude Code. Ships usage metadata to your organisation\\'s self-hosted receiver.',author:{name:'Psy-Fer'},category:'monitoring',source:{source:'git-subdir',url:'https://github.com/psy-fer/ccflux.git',path:'plugin',ref:'v0.1.0'},homepage:'https://github.com/psy-fer/ccflux'}]};
  writeJ(path.join(mktCp,'marketplace.json'),catalog);
  const km=readJ(knownJ)||{};
  km['ccflux']={source:{source:'github',repo:'psy-fer/ccflux'},installLocation:mktDir,lastUpdated:ts};
  writeJ(knownJ,km);
}

const ipJ=path.join(pdir,'installed_plugins.json');
let ip={version:2,plugins:{}};try{ip=JSON.parse(fs.readFileSync(ipJ,'utf8'));}catch(_){}
if(!ip.plugins)ip.plugins={};
ip.plugins['ccflux@ccflux']=[{scope:'user',installPath:dest,version:'0.1.0',installedAt:ts,lastUpdated:ts}];
fs.writeFileSync(ipJ,JSON.stringify(ip,null,2)+'\n');

const sJ=path.join(idir,'settings.json');
let s={};try{s=JSON.parse(fs.readFileSync(sJ,'utf8'));}catch(_){}
if(!s.enabledPlugins)s.enabledPlugins={};
s.enabledPlugins['ccflux@ccflux']=true;
if(offline){if(!s.extraKnownMarketplaces)s.extraKnownMarketplaces={};s.extraKnownMarketplaces['ccflux']={source:{source:'directory',path:mktDir}};}
fs.writeFileSync(sJ,JSON.stringify(s,null,2)+'\n');"
}

_reg_powershell() {
    local psbin="$1" install_dir="$2" plugin_dest="$3" now="$4"
    local tmp="/tmp/ccflux_install_$$.ps1"

    cat > "$tmp" << 'PSEOF'
$ErrorActionPreference = 'Stop'
$idir    = $env:CCFLUX_INSTALL_DIR
$dest    = $env:CCFLUX_PLUGIN_DEST
$ts      = $env:CCFLUX_TIMESTAMP
$offline = $env:CCFLUX_OFFLINE -eq 'true'
$pdir    = Join-Path $idir 'plugins'
$mktDir  = Join-Path $pdir 'marketplaces\ccflux'
$knownJ  = Join-Path $pdir 'known_marketplaces.json'

if ($offline) {
    New-Item -ItemType Directory -Path (Join-Path $mktDir '.claude-plugin') -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $mktDir 'plugins\ccflux')  -Force | Out-Null
    $pe = [ordered]@{
        name        = 'ccflux'
        description = "Per-turn token usage telemetry for Claude Code. Ships usage metadata to your organisation's self-hosted receiver."
        author      = @{name='Psy-Fer'}
        category    = 'monitoring'
        source      = [ordered]@{source='directory';path=$dest}
        homepage    = 'https://github.com/psy-fer/ccflux'
    }
    $catalog = [ordered]@{
        '$schema'   = 'https://anthropic.com/claude-code/marketplace.schema.json'
        name        = 'ccflux'
        description = 'ccflux — per-turn token usage telemetry for Claude Code'
        owner       = [ordered]@{name='Psy-Fer';email='j.ferguson@garvan.org.au'}
        plugins     = @($pe)
    }
    $catalog | ConvertTo-Json -Depth 10 | Set-Content (Join-Path $mktDir '.claude-plugin\marketplace.json') -Encoding UTF8
    if (Test-Path $knownJ) { $km = Get-Content $knownJ -Raw | ConvertFrom-Json }
    else { $km = [PSCustomObject]@{} }
    $km | Add-Member -NotePropertyName 'ccflux' -NotePropertyValue (
        [PSCustomObject]@{source=[PSCustomObject]@{source='directory';path=$mktDir};installLocation=$mktDir;lastUpdated=$ts}
    ) -Force
    $km | ConvertTo-Json -Depth 10 | Set-Content $knownJ -Encoding UTF8
} else {
    New-Item -ItemType Directory -Path (Join-Path $mktDir '.claude-plugin') -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $mktDir 'plugins\ccflux')  -Force | Out-Null
    $pe = [ordered]@{
        name        = 'ccflux'
        description = "Per-turn token usage telemetry for Claude Code. Ships usage metadata to your organisation's self-hosted receiver."
        author      = @{name='Psy-Fer'}
        category    = 'monitoring'
        source      = [ordered]@{source='git-subdir';url='https://github.com/psy-fer/ccflux.git';path='plugin';ref='v0.1.0'}
        homepage    = 'https://github.com/psy-fer/ccflux'
    }
    $catalog = [ordered]@{
        '$schema'   = 'https://anthropic.com/claude-code/marketplace.schema.json'
        name        = 'ccflux'
        description = 'ccflux — per-turn token usage telemetry for Claude Code'
        owner       = [ordered]@{name='Psy-Fer';email='j.ferguson@garvan.org.au'}
        plugins     = @($pe)
    }
    $catalog | ConvertTo-Json -Depth 10 | Set-Content (Join-Path $mktDir '.claude-plugin\marketplace.json') -Encoding UTF8
    if (Test-Path $knownJ) { $km = Get-Content $knownJ -Raw | ConvertFrom-Json }
    else { $km = [PSCustomObject]@{} }
    $km | Add-Member -NotePropertyName 'ccflux' -NotePropertyValue (
        [PSCustomObject]@{source=[PSCustomObject]@{source='github';repo='psy-fer/ccflux'};installLocation=$mktDir;lastUpdated=$ts}
    ) -Force
    $km | ConvertTo-Json -Depth 10 | Set-Content $knownJ -Encoding UTF8
}

$ipJ = Join-Path $pdir 'installed_plugins.json'
if (Test-Path $ipJ) { $ip = Get-Content $ipJ -Raw | ConvertFrom-Json }
else { $ip = [PSCustomObject]@{version=2;plugins=[PSCustomObject]@{}} }
if (-not $ip.PSObject.Properties['plugins']) { $ip | Add-Member -NotePropertyName 'plugins' -NotePropertyValue ([PSCustomObject]@{}) -Force }
$entry = @([PSCustomObject]@{scope='user';installPath=$dest;version='0.1.0';installedAt=$ts;lastUpdated=$ts})
$ip.plugins | Add-Member -NotePropertyName 'ccflux@ccflux' -NotePropertyValue $entry -Force
$ip | ConvertTo-Json -Depth 10 | Set-Content $ipJ -Encoding UTF8

$sJ = Join-Path $idir 'settings.json'
if (Test-Path $sJ) { $s = Get-Content $sJ -Raw | ConvertFrom-Json }
else { $s = [PSCustomObject]@{} }
if (-not $s.PSObject.Properties['enabledPlugins']) { $s | Add-Member -NotePropertyName 'enabledPlugins' -NotePropertyValue ([PSCustomObject]@{}) -Force }
$s.enabledPlugins | Add-Member -NotePropertyName 'ccflux@ccflux' -NotePropertyValue $true -Force
if ($offline) {
    if (-not $s.PSObject.Properties['extraKnownMarketplaces']) { $s | Add-Member -NotePropertyName 'extraKnownMarketplaces' -NotePropertyValue ([PSCustomObject]@{}) -Force }
    $s.extraKnownMarketplaces | Add-Member -NotePropertyName 'ccflux' -NotePropertyValue ([PSCustomObject]@{source=[PSCustomObject]@{source='directory';path=$mktDir}}) -Force
}
$s | ConvertTo-Json -Depth 10 | Set-Content $sJ -Encoding UTF8
PSEOF

    CCFLUX_INSTALL_DIR="$(_win_path "$install_dir")" \
    CCFLUX_PLUGIN_DEST="$(_win_path "$plugin_dest")" \
    CCFLUX_TIMESTAMP="$now" \
    CCFLUX_OFFLINE="${OFFLINE}" \
    "$psbin" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$(_win_path "$tmp")"
    rm -f "$tmp"
}

register_plugin() {
    local install_dir="$1" plugin_dest="$2"
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
        echo "  $(yellow "warning:") No JSON tool found (python/node/powershell) — plugin not registered."
        echo "           See the docs for manual registration steps."
        return
    fi

    "_reg_${tool}" "$bin" "$install_dir" "$plugin_dest" "$now"
    echo "  updated  plugins/known_marketplaces.json  (ccflux marketplace)"
    echo "  updated  plugins/installed_plugins.json   (ccflux@ccflux)"
    echo "  updated  settings.json                    (enabledPlugins: ccflux@ccflux)"
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

if [[ "$OFFLINE" == "true" ]]; then
    touch "${PLUGIN_DEST}/bin/.no-auto-download"
    echo "  created bin/.no-auto-download  $(dim "(auto-download disabled)")"
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
