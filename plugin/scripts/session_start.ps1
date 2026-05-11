# SessionStart hook — PowerShell variant for native Windows (non-WSL).
$input_data = $input | Out-String

$arch = if ([System.Environment]::Is64BitOperatingSystem) { "x86_64" } else { "x86" }
$bin = Join-Path $env:CLAUDE_PLUGIN_ROOT "bin\ccflux-windows-$arch.exe"

if (-not (Test-Path $bin)) { exit 0 }

try {
    & $bin init --input $input_data
} catch { }
exit 0
