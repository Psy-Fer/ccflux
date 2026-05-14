# SessionStart hook — PowerShell variant for native Windows (non-WSL).
$input_data = $input | Out-String

$arch     = if ([System.Environment]::Is64BitOperatingSystem) { "x86_64" } else { "x86" }
$bin_name = "ccflux-windows-$arch.exe"
$bin      = Join-Path $env:CLAUDE_PLUGIN_ROOT "bin\$bin_name"

$no_dl = Join-Path $env:CLAUDE_PLUGIN_ROOT "bin\.no-auto-download"
if (-not (Test-Path $bin) -and -not (Test-Path $no_dl)) {
    try {
        $plugin_json = Join-Path $env:CLAUDE_PLUGIN_ROOT ".claude-plugin\plugin.json"
        if (Test-Path $plugin_json) {
            $ver = (Get-Content $plugin_json -Raw | ConvertFrom-Json).version
            if ($ver) {
                $url     = "https://github.com/psy-fer/ccflux/releases/download/v$ver/$bin_name"
                $bin_dir = Join-Path $env:CLAUDE_PLUGIN_ROOT "bin"
                New-Item -ItemType Directory -Path $bin_dir -Force | Out-Null
                Invoke-WebRequest -Uri $url -OutFile $bin -TimeoutSec 60 -UseBasicParsing
            }
        }
    } catch { }
}

if (-not (Test-Path $bin)) { exit 0 }

try {
    & $bin init --input $input_data
} catch { }
exit 0
