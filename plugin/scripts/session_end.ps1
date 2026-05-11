# SessionEnd hook — PowerShell variant for native Windows (non-WSL).
# Runs the binary detached so CC doesn't kill it before it completes.
$input_data = $input | Out-String
$endpoint = $env:CLAUDE_PLUGIN_OPTION_API_ENDPOINT
$token    = $env:CLAUDE_PLUGIN_OPTION_API_TOKEN

$arch = if ([System.Environment]::Is64BitOperatingSystem) { "x86_64" } else { "x86" }
$bin = Join-Path $env:CLAUDE_PLUGIN_ROOT "bin\ccflux-windows-$arch.exe"

if (-not (Test-Path $bin)) { exit 0 }

$args_list = @("session-end", "--input", $input_data, "--endpoint", $endpoint, "--token", $token)
Start-Process -FilePath $bin -ArgumentList $args_list -WindowStyle Hidden -NoNewWindow
exit 0
