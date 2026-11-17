param([string]$Executable = (Join-Path $env:LOCALAPPDATA 'External Everything\bin\external-everything.exe'))
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class GlassWindowProbe {
 [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left,Top,Right,Bottom; }
 [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X,Y; }
 [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h,out RECT r);
 [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h,out RECT r);
 [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h,ref POINT p);
 [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
 [DllImport("user32.dll")] public static extern IntPtr SendMessageW(IntPtr h,uint m,IntPtr w,IntPtr l);
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
 [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr c);
 [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr h,int state);
 [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
 [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h,IntPtr after,int x,int y,int cx,int cy,uint flags);
 [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(POINT p);
 [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr h,uint flags);
 [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
 [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr h);
 [DllImport("dwmapi.dll")] public static extern int DwmFlush();
 public static IntPtr Pack(int x,int y){ return new IntPtr((y<<16)|(x&0xffff)); }
 public static void Frame(IntPtr h){ SendMessageW(h,0,IntPtr.Zero,IntPtr.Zero); DwmFlush(); SendMessageW(h,0,IntPtr.Zero,IntPtr.Zero); DwmFlush(); }
 public static POINT CaptionPoint(IntPtr h,double right){ RECT r;GetClientRect(h,out r);double s=GetDpiForWindow(h)/96.0;return new POINT{X=r.Right-(int)Math.Round(right*s),Y=(int)Math.Round(27*s)}; }
 public static int Hit(IntPtr h,int x,int y){ SendMessageW(h,0x0200,IntPtr.Zero,Pack(x,y));POINT p=new POINT{X=x,Y=y};ClientToScreen(h,ref p);return (int)SendMessageW(h,0x0084,IntPtr.Zero,Pack(p.X,p.Y)); }
 public static int CaptionHit(IntPtr h,double right){var p=CaptionPoint(h,right);return Hit(h,p.X,p.Y);}
 public static void ClickCaption(IntPtr h,double right,int expected){var p=CaptionPoint(h,right);int hit=Hit(h,p.X,p.Y);if(hit!=expected)throw new Exception("Caption hit mismatch: expected "+expected+", got "+hit);ClientToScreen(h,ref p);SendMessageW(h,0x00A1,new IntPtr(expected),Pack(p.X,p.Y));SendMessageW(h,0x00A2,new IntPtr(expected),Pack(p.X,p.Y));Frame(h);}
}
'@
Add-Type -AssemblyName System.Drawing
$exe = (Resolve-Path -LiteralPath $Executable).Path
$root = Split-Path -Parent $PSScriptRoot
$screenshots = Join-Path $root 'build\screenshots'
New-Item -ItemType Directory -Force -Path $screenshots | Out-Null
$oldDpi = [GlassWindowProbe]::SetThreadDpiAwarenessContext([IntPtr]::new(-4))
$previous = [GlassWindowProbe]::GetForegroundWindow()
$process = $null
try {
    $process = Start-Process -FilePath $exe -WorkingDirectory (Get-Location).Path -PassThru
    if (-not $process.WaitForInputIdle(20000)) { throw 'Glass window input loop failed' }
    $process.Refresh()
    $window = $process.MainWindowHandle
    if ($window -eq [IntPtr]::Zero -or -not [GlassWindowProbe]::IsWindow($window)) { throw 'Glass window handle is absent' }
    Write-Output "Window PID $($process.Id), title: $($process.MainWindowTitle)"
    [GlassWindowProbe]::SetForegroundWindow($window) | Out-Null
    if (-not [GlassWindowProbe]::SetWindowPos($window,[IntPtr]::new(-1),0,0,0,0,0x0053)) { throw 'Owned preview elevation failed' }
    [GlassWindowProbe]::Frame($window)
    $bounds = New-Object GlassWindowProbe+RECT
    [GlassWindowProbe]::GetWindowRect($window, [ref]$bounds) | Out-Null
    $center = New-Object GlassWindowProbe+POINT
    $center.X = [int](($bounds.Left+$bounds.Right)/2)
    $center.Y = [int](($bounds.Top+$bounds.Bottom)/2)
    if ([GlassWindowProbe]::GetAncestor([GlassWindowProbe]::WindowFromPoint($center),2) -ne $window) { throw 'Owned preview is occluded; screenshot skipped' }
    $bitmap = New-Object Drawing.Bitmap ($bounds.Right-$bounds.Left),($bounds.Bottom-$bounds.Top)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen($bounds.Left,$bounds.Top,0,0,$bitmap.Size)
        $path = Join-Path $screenshots ('external-everything-glass-'+[guid]::NewGuid().ToString('N')+'.png')
        $bitmap.Save($path,[Drawing.Imaging.ImageFormat]::Png)
        Write-Output "SCREENSHOT=$path"
    } finally { $graphics.Dispose(); $bitmap.Dispose() }
    foreach ($item in @(@('Minimize',109,8),@('Maximize',71,9),@('Close',33,20))) {
        $hit = [GlassWindowProbe]::CaptionHit($window,$item[1])
        Write-Output "$($item[0]) hit area: $hit"
        if ($hit -ne $item[2]) { throw "Incorrect native hit area for $($item[0])" }
    }
    $scale = [GlassWindowProbe]::GetDpiForWindow($window) / 96.0
    $drag = [GlassWindowProbe]::Hit($window,[int](180*$scale),[int](27*$scale))
    if ($drag -ne 2) { throw "Titlebar drag area is $drag rather than HTCAPTION" }
    Write-Output 'Native drag hit area verified.'
    [GlassWindowProbe]::ClickCaption($window,109,8)
    if (-not [GlassWindowProbe]::IsIconic($window)) { throw 'Minimize control failed' }
    Write-Output 'Minimize control verified.'
    [GlassWindowProbe]::ShowWindowAsync($window,9) | Out-Null
    [GlassWindowProbe]::SetForegroundWindow($window) | Out-Null
    [GlassWindowProbe]::Frame($window)
    [GlassWindowProbe]::ClickCaption($window,71,9)
    if (-not [GlassWindowProbe]::IsZoomed($window)) { throw 'Maximize control failed' }
    Write-Output 'Maximize control verified.'
    [GlassWindowProbe]::ClickCaption($window,71,9)
    if ([GlassWindowProbe]::IsZoomed($window)) { throw 'Restore-window control failed' }
    Write-Output 'Restore-window control verified.'
    [GlassWindowProbe]::ClickCaption($window,33,20)
    if (-not $process.WaitForExit(10000)) { throw 'Close control failed' }
    if ($process.ExitCode -ne 0) { throw "Glass window exited with $($process.ExitCode)" }
    Write-Output 'Close control verified; process exit 0.'
} finally {
    if ($process -and -not $process.HasExited) {
        if ($window -and [GlassWindowProbe]::IsWindow($window)) { [GlassWindowProbe]::SetWindowPos($window,[IntPtr]::new(-2),0,0,0,0,0x0013) | Out-Null }
        $process.CloseMainWindow() | Out-Null
        $process.WaitForExit(10000) | Out-Null
    }
    if ($previous -ne [IntPtr]::Zero) { [GlassWindowProbe]::SetForegroundWindow($previous) | Out-Null }
    [GlassWindowProbe]::SetThreadDpiAwarenessContext($oldDpi) | Out-Null
}
