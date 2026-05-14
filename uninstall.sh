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

die() { echo "$(printf "${_red}error:${_reset}") $*" >&2; exit 1; }

# ── Find installed locations ──────────────────────────────────────────────────

looks_like_claude_dir() {
    local dir="$1"
    [[ "$dir" == "$HOME" ]] && return 1
    [[ -f "${dir}/.claude.json" || -d "${dir}/projects" || -d "${dir}/plugins" ]]
}

find_installed_dirs() {
    local dirs=()
    local seen=()

    already_seen() {
        local d="$1" s
        for s in "${seen[@]:-}"; do [[ "$s" == "$d" ]] && return 0; done
        return 1
    }

    check_dir() {
        local dir="${1%/}"
        [[ -d "$dir" ]] || return 0
        already_seen "$dir" && return 0
        seen+=("$dir")
        looks_like_claude_dir "$dir" || return 0
        [[ -d "${dir}/plugins/${PLUGIN_NAME}" ]] && dirs+=("$dir")
    }

    check_dir "${HOME}/.claude"
    for dir in "${HOME}"/.claude-*/; do check_dir "$dir"; done
    while IFS= read -r json_file; do
        check_dir "$(dirname "$json_file")"
    done < <(find "${HOME}" -maxdepth 3 -name ".claude.json" 2>/dev/null | sort)

    printf '%s\n' "${dirs[@]:-}"
}

# ── Deregister from CC plugin registry ───────────────────────────────────────

deregister_plugin() {
    local install_dir="$1"
    local installed_json="${install_dir}/plugins/installed_plugins.json"
    local settings_json="${install_dir}/settings.json"

    local python_bin=""
    for p in python3 python; do
        command -v "$p" &>/dev/null && "$p" -c "import sys" 2>/dev/null && { python_bin="$p"; break; }
    done

    if [[ -z "$python_bin" ]]; then
        echo "  $(yellow "warning:") python not found — registry not updated."
        echo "           Remove $(dim "\"ccflux@local\"") from installed_plugins.json and settings.json manually."
        return
    fi

    if [[ -f "$installed_json" ]]; then
        CCFLUX_INSTALLED_JSON="$installed_json" \
        "$python_bin" - <<'PYEOF'
import json, os

path = os.environ['CCFLUX_INSTALLED_JSON']
with open(path) as f:
    data = json.load(f)
removed = data.get("plugins", {}).pop("ccflux@local", None)
if removed is not None:
    with open(path, "w") as f:
        json.dump(data, f, indent=2)
        f.write("\n")
    print("  updated  plugins/installed_plugins.json  (removed ccflux@local)")
PYEOF
    fi

    if [[ -f "$settings_json" ]]; then
        CCFLUX_SETTINGS_JSON="$settings_json" \
        "$python_bin" - <<'PYEOF'
import json, os

path = os.environ['CCFLUX_SETTINGS_JSON']
with open(path) as f:
    data = json.load(f)
removed = data.get("enabledPlugins", {}).pop("ccflux@local", None)
if removed is not None:
    with open(path, "w") as f:
        json.dump(data, f, indent=2)
        f.write("\n")
    print("  updated  settings.json  (removed ccflux@local)")
PYEOF
    fi
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

# Prompt for choice
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

# Ask about data directory
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
