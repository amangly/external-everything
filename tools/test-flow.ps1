param([string]$Executable = (Join-Path $env:LOCALAPPDATA 'External Everything\bin\external-everything.exe'))
$ErrorActionPreference = 'Stop'
$exe = (Resolve-Path -LiteralPath $Executable).Path
$start = New-Object Diagnostics.ProcessStartInfo
$start.FileName = $exe
$start.Arguments = '--inspect-flow'
$start.UseShellExecute = $false
$start.CreateNoWindow = $true
$start.RedirectStandardOutput = $true
$start.RedirectStandardError = $true
$process = New-Object Diagnostics.Process
$process.StartInfo = $start
try {
    if (-not $process.Start()) { throw 'Flow diagnostic start failed.' }
    $handle = $process.Handle
    if (-not $process.WaitForExit(10000)) { $process.Kill(); $process.WaitForExit(); throw 'Flow diagnostic timed out.' }
    $stdout = $process.StandardOutput.ReadToEnd()
    $stderr = $process.StandardError.ReadToEnd()
    if ($stdout.Length -gt 4096 -or $stderr.Length -gt 4096) { throw 'Flow diagnostic output exceeds its limit.' }
    if ($process.ExitCode -ne 0) { throw "Flow diagnostic failed: $stderr" }
    $flow = $stdout | ConvertFrom-Json
    if ($flow.stage -notin @('games', 'restart_required', 'setup_required')) { throw 'Flow stage is unsupported.' }
    if ($flow.stage -eq 'games' -and (!$flow.cr3 -or [uint64]$flow.cr3 -eq 0)) { throw 'Game selection requires a live attachment CR3.' }
    if ($flow.stage -eq 'restart_required' -and -not $flow.installed) { throw 'Restart state requires verified installation state.' }
    Write-Output $stdout.Trim()
    Write-Output "Verified startup route: $($flow.stage)"
} finally { $process.Dispose() }
