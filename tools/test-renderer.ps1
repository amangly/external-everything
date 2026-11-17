param(
    [string]$Runtime = (Join-Path $env:LOCALAPPDATA 'External Everything\bin\games\pointblank\external-runtime.exe'),
    [switch]$CloseMenu
)
$ErrorActionPreference = 'Stop'
$exe = (Resolve-Path -LiteralPath $Runtime).Path
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System; using System.Text; using System.Runtime.InteropServices;
public static class ExternalRendererProbe {
 public delegate bool EnumProc(IntPtr h, IntPtr p);
 [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L,T,R,B; }
 [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc fn, IntPtr p);
 [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
 [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
 [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
 [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr c);
 [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h,uint m,IntPtr w,IntPtr l);
 [DllImport("dwmapi.dll")] public static extern int DwmFlush();
 public static IntPtr Menu(uint id) { IntPtr result=IntPtr.Zero; EnumWindows((h,p)=>{uint pid;GetWindowThreadProcessId(h,out pid);if(pid==id&&IsWindowVisible(h)){var text=new StringBuilder(256);GetClassNameW(h,text,256);if(text.ToString().IndexOf("Imgui",StringComparison.OrdinalIgnoreCase)>=0){result=h;return false;}}return true;},IntPtr.Zero);return result; }
}
'@
$nonce = [guid]::NewGuid().ToString('N')
$eventPrefix = 'Local\ExternalEverything-runtime-render-test-' + $nonce
$created = $false
$started = [Threading.EventWaitHandle]::new($false, [Threading.EventResetMode]::ManualReset, ($eventPrefix + '-started'), [ref]$created)
$ready = [Threading.EventWaitHandle]::new($false, [Threading.EventResetMode]::ManualReset, ($eventPrefix + '-ready'), [ref]$created)
$start = New-Object Diagnostics.ProcessStartInfo
$start.FileName = $exe
$start.Arguments = '--render-smoke 8'
$start.UseShellExecute = $false
$start.CreateNoWindow = $true
$start.RedirectStandardOutput = $true
$start.RedirectStandardError = $true
$start.EnvironmentVariables['EXTERNAL_EVERYTHING_STARTED_EVENT'] = $eventPrefix + '-started'
$start.EnvironmentVariables['EXTERNAL_EVERYTHING_READY_EVENT'] = $eventPrefix + '-ready'
$process = New-Object Diagnostics.Process
$process.StartInfo = $start
$oldDpi = [ExternalRendererProbe]::SetThreadDpiAwarenessContext([IntPtr]::new(-4))
try {
    if (-not $process.Start()) { throw 'Renderer smoke process start failed.' }
    $handle = $process.Handle
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $stderr = $process.StandardError.ReadToEndAsync()
    if (-not $started.WaitOne(8000)) { throw 'Renderer started event timed out.' }
    if (-not $ready.WaitOne(20000)) { throw 'Renderer ready event timed out.' }
    $menu = [ExternalRendererProbe]::Menu([uint32]$process.Id)
    if ($menu -eq [IntPtr]::Zero) { throw 'Owned ImGui menu surface is absent.' }
    [ExternalRendererProbe]::DwmFlush() | Out-Null
    $rect = New-Object ExternalRendererProbe+RECT
    if (-not [ExternalRendererProbe]::GetWindowRect($menu,[ref]$rect)) { throw 'Read renderer surface bounds failed.' }
    # Chrome's copied default is position (100,64), width 780, max height 860.
    # Capture the menu region only, rather than the game's surrounding player UI.
    $left = $rect.L + 96
    $top = $rect.T + 60
    $width = [Math]::Min(788, $rect.R-$left)
    $height = [Math]::Min(868, $rect.B-$top)
    if ($width -le 0 -or $height -le 0) { throw 'ImGui menu screenshot bounds are empty.' }
    $bitmap = New-Object Drawing.Bitmap $width,$height
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen($left,$top,0,0,$bitmap.Size)
        $directory = Join-Path (Split-Path -Parent $PSScriptRoot) 'build\screenshots'
        New-Item -ItemType Directory -Force -Path $directory | Out-Null
        $image = Join-Path $directory ('pointblank-palepale-menu-'+$nonce+'.png')
        $bitmap.Save($image,[Drawing.Imaging.ImageFormat]::Png)
        Write-Output "SCREENSHOT=$image"
    } finally { $graphics.Dispose(); $bitmap.Dispose() }
    if ($CloseMenu) {
        if (-not [ExternalRendererProbe]::PostMessageW($menu,0x0010,[IntPtr]::Zero,[IntPtr]::Zero)) { throw 'Native menu close request failed.' }
    }
    if (-not $process.WaitForExit(20000)) { throw 'Bounded renderer smoke failed to finish.' }
    $output = $stdout.Result
    $errorText = $stderr.Result
    if ($process.ExitCode -ne 0) { throw "Renderer smoke failed: $errorText" }
    $report = $output | ConvertFrom-Json
    if (-not $report.d3d11_ready -or -not $report.d2d_ready -or $report.frames -lt 1 -or $report.menu_frames -lt 1) { throw 'Native render report lacks successful frames.' }
    $evidence = Join-Path (Split-Path -Parent $PSScriptRoot) ('build\renderer-verification-'+$nonce+'.json')
    $file = [IO.File]::Open($evidence,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
    try { $bytes=[Text.UTF8Encoding]::new($false).GetBytes($output); $file.Write($bytes,0,$bytes.Length); $file.Flush($true) } finally { $file.Dispose() }
    Write-Output "EVIDENCE=$evidence"
    Write-Output $output.Trim()
    Write-Output 'Verified native render events, owned ImGui surface, frame rendering, and clean bounded shutdown.'
} finally {
    if ($process.Id -and -not $process.HasExited) { $process.Kill(); $process.WaitForExit() }
    $process.Dispose()
    $started.Dispose()
    $ready.Dispose()
    [ExternalRendererProbe]::SetThreadDpiAwarenessContext($oldDpi) | Out-Null
}
