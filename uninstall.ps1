#Requires -Version 5.1
# ccflux plugin uninstaller — native Windows PowerShell
#
# Run from the ccflux repo root (or anywhere):
#   .\uninstall.ps1

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$PluginName = 'ccflux'

# ── Colour helpers ────────────────────────────────────────────────────────────

function Write-Bold   ($msg) { Write-Host $msg -ForegroundColor White }
function Write-Green  ($msg) { Write-Host $msg -ForegroundColor Green }
function Write-Yellow ($msg) { Write-Host $msg -ForegroundColor Yellow }
function Write-Dim    ($msg) { Write-Host $msg -ForegroundColor DarkGray }

# ── Find installed locations ──────────────────────────────────────────────────

function Test-ClaudeDir ([string]$Dir) {
    if ($Dir -eq $env:USERPROFILE) { return $false }
    return (Test-Path (Join-Path $Dir '.claude.json')  -PathType Leaf)      -or
           (Test-Path (Join-Path $Dir 'projects')       -PathType Container) -or
           (Test-Path (Join-Path $Dir 'plugins')        -PathType Container)
}

function Find-InstalledDirs {
    $seen   = [System.Collections.Generic.HashSet[string]]::new(
                  [System.StringComparer]::OrdinalIgnoreCase)
    $result = @()

    $raw = @()
    $raw += Join-Path $env:USERPROFILE '.claude'
    Get-ChildItem -Path $env:USERPROFILE -Directory -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -like '.claude-*' } |
        ForEach-Object { $raw += $_.FullName }
    Get-ChildItem -Path $env:USERPROFILE -Filter '.claude.json' `
                  -Recurse -Depth 2 -ErrorAction SilentlyContinue |
        ForEach-Object { $raw += $_.DirectoryName }

    foreach ($dir in $raw) {
        $dir = $dir.TrimEnd('\', '/')
        if (-not (Test-Path $dir -PathType Container)) { continue }
        if (-not $seen.Add($dir))                      { continue }
        if (-not (Test-ClaudeDir $dir))                { continue }
        if (Test-Path (Join-Path $dir "plugins\$PluginName") -PathType Container) {
            $result += $dir
        }
    }
    return $result
}

# ── Deregister from CC plugin registry ───────────────────────────────────────

function Deregister-Plugin ([string]$InstallDir) {
    $InstalledJson = Join-Path $InstallDir "plugins\installed_plugins.json"
    $SettingsJson  = Join-Path $InstallDir "settings.json"

    if (Test-Path $InstalledJson) {
        $ipData = Get-Content $InstalledJson -Raw | ConvertFrom-Json
        if ($null -ne $ipData.plugins.PSObject.Properties['ccflux@local']) {
            $ipData.plugins.PSObject.Properties.Remove('ccflux@local')
            $ipData | ConvertTo-Json -Depth 10 | Set-Content $InstalledJson -Encoding UTF8
            Write-Host "  updated  plugins\installed_plugins.json  (removed ccflux@local)"
        }
    }

    if (Test-Path $SettingsJson) {
        $settings = Get-Content $SettingsJson -Raw | ConvertFrom-Json
        if ($null -ne $settings.PSObject.Properties['enabledPlugins'] -and
            $null -ne $settings.enabledPlugins.PSObject.Properties['ccflux@local']) {
            $settings.enabledPlugins.PSObject.Properties.Remove('ccflux@local')
            $settings | ConvertTo-Json -Depth 10 | Set-Content $SettingsJson -Encoding UTF8
            Write-Host "  updated  settings.json  (removed ccflux@local)"
        }
    }
}

# ── Main ──────────────────────────────────────────────────────────────────────

Write-Host ""
Write-Host "ccflux plugin uninstaller (PowerShell)" -ForegroundColor White
Write-Host ""

$candidates = @(Find-InstalledDirs)

if ($candidates.Count -eq 0) {
    Write-Host "No ccflux installations found."
    Write-Host ""
    exit 0
}

Write-Host "Found ccflux installation(s):"
Write-Host ""
for ($i = 0; $i -lt $candidates.Count; $i++) {
    $dir  = $candidates[$i]
    $note = ''
    if ($dir -eq (Join-Path $env:USERPROFILE '.claude')) { $note = '  (default)' }
    Write-Host ("  {0}) {1}{2}" -f ($i + 1), $dir, $note)
}
if ($candidates.Count -gt 1) {
    Write-Host "  a) All of the above"
}
Write-Host ""

# Prompt for choice
$toRemove = @()
while ($toRemove.Count -eq 0) {
    $choice = Read-Host "Choose installation to remove [1]"
    if ([string]::IsNullOrWhiteSpace($choice)) { $choice = '1' }

    if ($candidates.Count -gt 1 -and $choice -match '^[aA]$') {
        $toRemove = $candidates
    } elseif ($choice -match '^\d+$') {
        $idx = [int]$choice - 1
        if ($idx -ge 0 -and $idx -lt $candidates.Count) {
            $toRemove = @($candidates[$idx])
        } else {
            Write-Yellow "Invalid choice."
        }
    } else {
        Write-Yellow "Invalid choice."
    }
}

# Ask about data
Write-Host ""
$removeData = Read-Host "Also remove ccflux data (signing key, token cache, pending queue)? [y/N]"

Write-Host ""

foreach ($installDir in $toRemove) {
    $pluginDir = Join-Path $installDir "plugins\$PluginName"
    $dataDir   = Join-Path $installDir $PluginName

    Write-Bold "Removing from: $installDir"

    Remove-Item $pluginDir -Recurse -Force
    Write-Host "  removed  plugins\$PluginName\"

    Deregister-Plugin $installDir

    if ($removeData -match '^[Yy]$' -and (Test-Path $dataDir -PathType Container)) {
        Remove-Item $dataDir -Recurse -Force
        Write-Host "  removed  $PluginName\  (data)"
    } elseif (Test-Path $dataDir -PathType Container) {
        Write-Dim  "  kept     $PluginName\  (data preserved — remove manually if desired)"
    }

    Write-Host ""
}

Write-Green "Done!  Restart Claude Code for the change to take effect."
Write-Host ""
