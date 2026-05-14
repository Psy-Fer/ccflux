#Requires -Version 5.1
# ccflux plugin installer — native Windows PowerShell
#
# Run from the ccflux repo root:
#   .\install.ps1
#
# Optional switch:
#   -UseStandardHooks   Install hooks.json instead of hooks-windows.json.
#                       Use this if Claude Code runs via Git Bash or WSL
#                       rather than native PowerShell.

param(
    [switch]$UseStandardHooks,
    [switch]$Offline
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$PluginName = 'ccflux'
$ScriptDir  = $PSScriptRoot
$PluginSrc  = Join-Path $ScriptDir 'plugin'

# ── Colour helpers ────────────────────────────────────────────────────────────

function Write-Bold   ($msg) { Write-Host $msg -ForegroundColor White }
function Write-Green  ($msg) { Write-Host $msg -ForegroundColor Green }
function Write-Yellow ($msg) { Write-Host $msg -ForegroundColor Yellow }
function Write-Dim    ($msg) { Write-Host $msg -ForegroundColor DarkGray }
function Die          ($msg) { Write-Host "error: $msg" -ForegroundColor Red; exit 1 }

# ── Find candidate CC data directories ───────────────────────────────────────

function Test-ClaudeDir ([string]$Dir) {
    # Home itself is never a CC data dir
    if ($Dir -eq $env:USERPROFILE) { return $false }
    return (Test-Path (Join-Path $Dir '.claude.json')  -PathType Leaf)      -or
           (Test-Path (Join-Path $Dir 'projects')       -PathType Container) -or
           (Test-Path (Join-Path $Dir 'plugins')        -PathType Container)
}

function Find-ClaudeDirs {
    $seen = [System.Collections.Generic.HashSet[string]]::new(
                [System.StringComparer]::OrdinalIgnoreCase)
    $result = @()

    # Collect raw candidates
    $raw = @()

    # Standard default
    $raw += Join-Path $env:USERPROFILE '.claude'

    # Any .claude-* siblings in USERPROFILE
    Get-ChildItem -Path $env:USERPROFILE -Directory -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -like '.claude-*' } |
        ForEach-Object { $raw += $_.FullName }

    # Scan up to 2 levels deep for .claude.json (catches CLAUDE_CONFIG_DIR paths)
    Get-ChildItem -Path $env:USERPROFILE -Filter '.claude.json' `
                  -Recurse -Depth 2 -ErrorAction SilentlyContinue |
        ForEach-Object { $raw += $_.DirectoryName }

    foreach ($dir in $raw) {
        $dir = $dir.TrimEnd('\', '/')
        if (-not (Test-Path $dir -PathType Container)) { continue }
        if (-not $seen.Add($dir))                      { continue }
        if (Test-ClaudeDir $dir) { $result += $dir }
    }

    return $result
}

# ── Main ──────────────────────────────────────────────────────────────────────

Write-Host ""
Write-Host "ccflux plugin installer (PowerShell)" -ForegroundColor White
Write-Host ""

# Preflight: plugin source
if (-not (Test-Path $PluginSrc -PathType Container)) {
    Die "plugin\ directory not found — run this script from the ccflux repo root."
}
foreach ($sub in @('.claude-plugin', 'scripts', 'hooks')) {
    if (-not (Test-Path (Join-Path $PluginSrc $sub) -PathType Container)) {
        Die "plugin\$sub not found — check your checkout."
    }
}

# Preflight: binaries
$BinDir   = Join-Path $PluginSrc 'bin'
$BinCount = 0
if (Test-Path $BinDir -PathType Container) {
    $BinCount = @(Get-ChildItem $BinDir -File -ErrorAction SilentlyContinue).Count
}
if ($BinCount -eq 0) {
    Write-Yellow "warning: No binaries found in plugin\bin\"
    Write-Host   "         Download a release and unpack the Windows binary into plugin\bin\,"
    Write-Dim    "         or build: cd ccflux-core; cargo build --release --target x86_64-pc-windows-msvc"
    Write-Host ""
    $ans = Read-Host "Continue anyway (you can add the binary after installing)? [y/N]"
    if ($ans -notmatch '^[Yy]$') { exit 0 }
    Write-Host ""
}

# ── Discover install targets ──────────────────────────────────────────────────

Write-Host "Scanning for Claude Code data directories..."
Write-Host ""

$candidates = @(Find-ClaudeDirs)

if ($candidates.Count -eq 0) {
    Write-Yellow "No Claude Code directories found automatically."
    Write-Host ""
}

if ($candidates.Count -gt 0) {
    Write-Host "Found Claude Code installation(s):"
    Write-Host ""
    for ($i = 0; $i -lt $candidates.Count; $i++) {
        $dir  = $candidates[$i]
        $note = ''
        if ($dir -eq (Join-Path $env:USERPROFILE '.claude')) { $note += '  (default)' }
        if (Test-Path (Join-Path $dir "plugins\$PluginName") -PathType Container) {
            $note += '  (already installed)'
        }
        Write-Host ("  {0}) {1}{2}" -f ($i + 1), $dir, $note)
    }
    Write-Host "  c) Enter a custom path"
    Write-Host ""
}

# Prompt loop
$installDir = $null
while ($null -eq $installDir) {
    if ($candidates.Count -gt 0) {
        $choice = Read-Host "Choose installation target [1]"
        if ([string]::IsNullOrWhiteSpace($choice)) { $choice = '1' }
    } else {
        $choice = Read-Host "Enter path (e.g. $env:USERPROFILE\.claude)"
    }

    if ($choice -match '^[cC]$') {
        $custom = Read-Host "Path"
        $custom = $custom.Trim('"', "'").Trim()
        if ([string]::IsNullOrWhiteSpace($custom)) { Write-Yellow "Path cannot be empty."; continue }
        $installDir = $custom
    } elseif ($candidates.Count -eq 0) {
        # Their input was the path
        $custom = $choice.Trim('"', "'").Trim()
        if ([string]::IsNullOrWhiteSpace($custom)) { Write-Yellow "Path cannot be empty."; continue }
        $installDir = $custom
    } elseif ($choice -match '^\d+$') {
        $idx = [int]$choice - 1
        if ($idx -ge 0 -and $idx -lt $candidates.Count) {
            $installDir = $candidates[$idx]
        } else {
            Write-Yellow "Invalid choice — enter a number between 1 and $($candidates.Count), or 'c'."
        }
    } else {
        Write-Yellow "Invalid choice — enter a number between 1 and $($candidates.Count), or 'c'."
    }
}

$PluginDest = Join-Path $installDir "plugins\$PluginName"

Write-Host ""
Write-Bold "Installing to: $PluginDest"

# ── Hooks selection ───────────────────────────────────────────────────────────

if ($UseStandardHooks) {
    $HooksSrc = Join-Path $PluginSrc 'hooks\hooks.json'
    Write-Dim "  Using hooks.json (Git Bash / WSL mode)"
} else {
    $HooksSrc = Join-Path $PluginSrc 'hooks\hooks-windows.json'
    Write-Dim "  Using hooks-windows.json (PowerShell mode)"
    Write-Dim "  Tip: re-run with -UseStandardHooks if CC runs via Git Bash or WSL."
}

# ── Copy files ────────────────────────────────────────────────────────────────

Write-Host ""

# .claude-plugin/
$dest = Join-Path $PluginDest '.claude-plugin'
New-Item -ItemType Directory -Path $dest -Force | Out-Null
Copy-Item (Join-Path $PluginSrc '.claude-plugin\plugin.json') $dest -Force
Write-Host "  copied  .claude-plugin\plugin.json"

# hooks/ — install chosen variant as hooks.json
$dest = Join-Path $PluginDest 'hooks'
New-Item -ItemType Directory -Path $dest -Force | Out-Null
Copy-Item $HooksSrc (Join-Path $dest 'hooks.json') -Force
Write-Host "  copied  hooks\hooks.json"

# scripts/
$dest = Join-Path $PluginDest 'scripts'
New-Item -ItemType Directory -Path $dest -Force | Out-Null
Get-ChildItem (Join-Path $PluginSrc 'scripts') -File | ForEach-Object {
    Copy-Item $_.FullName $dest -Force
    Write-Host "  copied  scripts\$($_.Name)"
}

# bin/
$dest = Join-Path $PluginDest 'bin'
New-Item -ItemType Directory -Path $dest -Force | Out-Null
if ($BinCount -gt 0) {
    Get-ChildItem $BinDir -File | ForEach-Object {
        Copy-Item $_.FullName $dest -Force
        Write-Host "  copied  bin\$($_.Name)"
    }
} else {
    Write-Yellow "  created bin\  (empty — add ccflux-windows-x86_64.exe before using)"
}

if ($Offline) {
    New-Item -ItemType File -Path (Join-Path $dest '.no-auto-download') -Force | Out-Null
    Write-Dim   "  created bin\.no-auto-download  (auto-download disabled)"
}

# ── Register plugin in CC's plugin registry ───────────────────────────────────

function Register-Plugin ([string]$InstallDir, [string]$PluginDest) {
    $PluginsDir   = Join-Path $InstallDir "plugins"
    $InstalledJson = Join-Path $PluginsDir "installed_plugins.json"
    $SettingsJson  = Join-Path $InstallDir "settings.json"
    $KnownJson     = Join-Path $PluginsDir "known_marketplaces.json"
    $MktDir        = Join-Path $PluginsDir "marketplaces\ccflux"
    $Now = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ss.fffZ")

    # Marketplace catalog
    New-Item -ItemType Directory -Path (Join-Path $MktDir '.claude-plugin') -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $MktDir 'plugins\ccflux')  -Force | Out-Null
    $catalog = [ordered]@{
        '$schema'   = 'https://anthropic.com/claude-code/marketplace.schema.json'
        name        = 'ccflux'
        description = 'ccflux — per-turn token usage telemetry for Claude Code'
        owner       = [ordered]@{name='Psy-Fer';email='j.ferguson@garvan.org.au'}
        plugins     = @([ordered]@{
            name        = 'ccflux'
            description = "Per-turn token usage telemetry for Claude Code. Ships usage metadata to your organisation's self-hosted receiver."
            author      = @{name='Psy-Fer'}
            category    = 'monitoring'
            source      = [ordered]@{source='git-subdir';url='https://github.com/psy-fer/ccflux.git';path='plugin';ref='v0.1.0'}
            homepage    = 'https://github.com/psy-fer/ccflux'
        })
    }
    $catalog | ConvertTo-Json -Depth 10 | Set-Content (Join-Path $MktDir '.claude-plugin\marketplace.json') -Encoding UTF8
    Write-Host "  updated  plugins\marketplaces\ccflux\  (ccflux marketplace)"

    # known_marketplaces.json
    if (Test-Path $KnownJson) { $km = Get-Content $KnownJson -Raw | ConvertFrom-Json }
    else { $km = [PSCustomObject]@{} }
    $km | Add-Member -NotePropertyName 'ccflux' -NotePropertyValue (
        [PSCustomObject]@{source=[PSCustomObject]@{source='github';repo='psy-fer/ccflux'};installLocation=$MktDir;lastUpdated=$Now}
    ) -Force
    $km | ConvertTo-Json -Depth 10 | Set-Content $KnownJson -Encoding UTF8
    Write-Host "  updated  plugins\known_marketplaces.json  (ccflux marketplace)"

    # installed_plugins.json
    if (Test-Path $InstalledJson) { $ipData = Get-Content $InstalledJson -Raw | ConvertFrom-Json }
    else { $ipData = [PSCustomObject]@{ version = 2; plugins = [PSCustomObject]@{} } }
    if ($null -eq $ipData.PSObject.Properties['plugins']) {
        $ipData | Add-Member -NotePropertyName 'plugins' -NotePropertyValue ([PSCustomObject]@{}) -Force
    }
    $entry = @([PSCustomObject]@{ scope='user'; installPath=$PluginDest; version='0.1.0'; installedAt=$Now; lastUpdated=$Now })
    $ipData.plugins | Add-Member -NotePropertyName 'ccflux@ccflux' -NotePropertyValue $entry -Force
    $ipData | ConvertTo-Json -Depth 10 | Set-Content $InstalledJson -Encoding UTF8
    Write-Host "  updated  plugins\installed_plugins.json  (ccflux@ccflux)"

    # settings.json
    if (Test-Path $SettingsJson) { $settings = Get-Content $SettingsJson -Raw | ConvertFrom-Json }
    else { $settings = [PSCustomObject]@{} }
    if ($null -eq $settings.PSObject.Properties['enabledPlugins']) {
        $settings | Add-Member -NotePropertyName 'enabledPlugins' -NotePropertyValue ([PSCustomObject]@{}) -Force
    }
    $settings.enabledPlugins | Add-Member -NotePropertyName 'ccflux@ccflux' -NotePropertyValue $true -Force
    $settings | ConvertTo-Json -Depth 10 | Set-Content $SettingsJson -Encoding UTF8
    Write-Host "  updated  settings.json  (enabledPlugins: ccflux@ccflux)"
}

Write-Host ""
Register-Plugin $installDir $PluginDest

# ── Done ──────────────────────────────────────────────────────────────────────

Write-Host ""
Write-Green "Done!  Plugin installed to:"
Write-Host  "       $PluginDest"
Write-Host ""
Write-Bold "Next steps:"
Write-Host ""
Write-Host "  1. Restart Claude Code (or run /plugins refresh if supported)."
Write-Host ""
Write-Host "  2. Open Claude Code settings -> Plugins -> ccflux and set:"
Write-Host "         Receiver endpoint   your organisation's ccflux receiver URL" -ForegroundColor White
Write-Host "         API token           your personal token, provided by IT"     -ForegroundColor White
Write-Host ""
Write-Dim  "     Or create <CC data dir>\ccflux\config.json:"
Write-Dim  '       { "endpoint": "https://ccflux.example.org", "token": "rtok_..." }'
Write-Host ""
Write-Host "  3. Start a session — the first turn will register your device key"
Write-Host "     and begin reporting usage."
Write-Host ""
if ($BinCount -eq 0) {
    Write-Yellow "Remember: download ccflux-windows-x86_64.exe from the latest release and place it in:"
    Write-Host   "  $PluginDest\bin\"
    Write-Host ""
}
