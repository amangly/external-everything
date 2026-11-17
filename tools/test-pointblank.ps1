param([string]$Launcher = (Join-Path $env:LOCALAPPDATA 'External Everything\bin\external-everything.exe'))
$ErrorActionPreference = 'Stop'
$launcherPath = (Resolve-Path -LiteralPath $Launcher).Path
$runtimePath = Join-Path (Split-Path -Parent $launcherPath) 'games\pointblank\external-runtime.exe'
if (-not (Test-Path -LiteralPath $runtimePath -PathType Leaf)) { throw 'Packaged PointBlank runtime is absent.' }
function Invoke-Json([string]$File, [string]$Arguments, [int]$Timeout = 30000) {
    $start = New-Object Diagnostics.ProcessStartInfo
    $start.FileName = $File
    $start.Arguments = $Arguments
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $process = New-Object Diagnostics.Process
    $process.StartInfo = $start
    try {
        if (-not $process.Start()) { throw 'Native test process start failed.' }
        $handle = $process.Handle
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit($Timeout)) { $process.Kill(); $process.WaitForExit(); throw "Native test timed out: $Arguments" }
        $stdout = $stdoutTask.Result
        $stderr = $stderrTask.Result
        if ($stdout.Length -gt 65536 -or $stderr.Length -gt 65536) { throw 'Native test output exceeds its limit.' }
        if ($process.ExitCode -ne 0) { throw "Native test failed: $Arguments : $stderr" }
        $result = $stdout | ConvertFrom-Json
        Write-Host $stdout.Trim()
        return $result
    } finally { $process.Dispose() }
}
$contract = Invoke-Json $runtimePath 'contract --json'
if ($contract.schema -ne 1 -or $contract.backend_abi -ne 'hyper-rev/cpuid-8a3b55c7927/v1' -or @($contract.games | Where-Object { $_.id -eq 'pointblank' }).Count -ne 1) { throw 'PointBlank runtime contract differs.' }
$memory = Invoke-Json $runtimePath '--self-test'
if (-not $memory.attachment -or $memory.memory_read_write -ne 'verified' -or $memory.process_discovery -ne 'verified') { throw 'Native hypercall memory/process verification differs.' }
$snapshot = Invoke-Json $runtimePath '--snapshot'
if ($snapshot.game -ne 'pointblank') { throw 'PointBlank snapshot identity differs.' }
if ($snapshot.pid -and $snapshot.status -eq 'PointBlank check failed') { throw "PointBlank capture failed: $($snapshot.detail)" }
$render = Invoke-Json $runtimePath '--render-smoke 5' 30000
if (-not $render.d3d11_ready -or -not $render.d2d_ready -or $render.frames -lt 1 -or $render.menu_frames -lt 1) { throw 'Palepale Direct3D/ImGui rendering verification failed.' }
if ($render.reader_hz -lt 30) { throw 'Live PointBlank reader update rate remains below target.' }
$performance = Join-Path (Split-Path -Parent $PSScriptRoot) ('build\pointblank-performance-'+[guid]::NewGuid().ToString('N')+'.json')
$file=[IO.File]::Open($performance,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
try { $bytes=[Text.UTF8Encoding]::new($false).GetBytes(($render|ConvertTo-Json -Depth 5)); $file.Write($bytes,0,$bytes.Length); $file.Flush($true) } finally { $file.Dispose() }
Write-Output "PERFORMANCE=$performance"
$launch = Invoke-Json $launcherPath '--verify-runtime-launch' 40000
if (-not $launch.started -or -not $launch.window_ready -or $launch.game -ne 'pointblank') { throw 'Launcher-to-runtime event acknowledgements failed.' }
if (Get-Process -Id $launch.runtime_pid -ErrorAction SilentlyContinue) { throw 'Launcher-owned test runtime survived session cleanup.' }
Write-Output 'Verified contract, hypercall memory/process discovery, Palepale Direct2D ESP + Direct3D11/ImGui menu, launcher readiness handshake, and child cleanup.'
Write-Output "PointBlank capture state: $($snapshot.status)"
if ($snapshot.detail) { Write-Output $snapshot.detail }
