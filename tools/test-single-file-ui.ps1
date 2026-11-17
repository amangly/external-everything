param([string]$Executable=(Join-Path (Split-Path -Parent $PSScriptRoot) 'dist\external-everything.exe'))
$ErrorActionPreference='Stop'
$root=Split-Path -Parent $PSScriptRoot
$directory=Join-Path $root ('build\single-file-ui-'+[guid]::NewGuid().ToString('N'))
$app=Join-Path $directory 'app';$scratch=Join-Path $directory 'temp';$local=Join-Path $directory 'local'
New-Item -ItemType Directory -Path $app,$scratch,$local|Out-Null
$exe=Join-Path $app 'external-everything.exe'
Copy-Item -LiteralPath (Resolve-Path -LiteralPath $Executable).Path -Destination $exe
Add-Type -TypeDefinition @'
using System;using System.Text;using System.Runtime.InteropServices;
public static class SingleFileUI{
 public delegate bool Callback(IntPtr h,IntPtr p);
 [StructLayout(LayoutKind.Sequential)]public struct RECT{public int L,T,R,B;}
 [DllImport("user32.dll")]public static extern bool GetClientRect(IntPtr h,out RECT r);
 [DllImport("user32.dll")]public static extern uint GetDpiForWindow(IntPtr h);
 [DllImport("user32.dll")]public static extern IntPtr SendMessageW(IntPtr h,uint m,IntPtr w,IntPtr l);
 [DllImport("user32.dll")]public static extern bool PostMessageW(IntPtr h,uint m,IntPtr w,IntPtr l);
 [DllImport("user32.dll")]public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr c);
 [DllImport("user32.dll")]public static extern bool EnumWindows(Callback cb,IntPtr p);
 [DllImport("user32.dll")]public static extern uint GetWindowThreadProcessId(IntPtr h,out uint pid);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)]public static extern int GetClassNameW(IntPtr h,StringBuilder s,int n);
 public static bool RenderWindow(uint id){bool found=false;EnumWindows((h,p)=>{uint owner;GetWindowThreadProcessId(h,out owner);if(owner==id){var s=new StringBuilder(256);GetClassNameW(h,s,256);if(s.ToString().IndexOf("Imgui",StringComparison.OrdinalIgnoreCase)>=0)found=true;}return !found;},IntPtr.Zero);return found;}
 public static IntPtr Point(int x,int y){return new IntPtr((y<<16)|(x&65535));}
}
'@
$s=New-Object Diagnostics.ProcessStartInfo
$s.FileName=$exe;$s.WorkingDirectory=$app;$s.UseShellExecute=$false
$s.EnvironmentVariables['TEMP']=$scratch;$s.EnvironmentVariables['TMP']=$scratch;$s.EnvironmentVariables['LOCALAPPDATA']=$local
$p=New-Object Diagnostics.Process;$p.StartInfo=$s
$old=[SingleFileUI]::SetThreadDpiAwarenessContext([IntPtr]::new(-4));$child=$null
try{
 if(-not $p.Start()){throw 'Single-file GUI failed to start'}
 $handle=$p.Handle
 if(-not $p.WaitForInputIdle(20000)){throw 'Single-file GUI input loop failed'}
 $deadline=[datetime]::UtcNow.AddSeconds(10)
 do{$p.Refresh();if($p.MainWindowHandle -ne [IntPtr]::Zero -and $p.MainWindowTitle.EndsWith('Games')){break};if($p.HasExited){throw 'Single-file GUI exited before Games'};[Threading.Thread]::Sleep(20)}while([datetime]::UtcNow -lt $deadline)
 if(-not $p.MainWindowTitle.EndsWith('Games')){throw 'Single-file GUI did not reach Games'}
 $window=$p.MainWindowHandle;$r=New-Object SingleFileUI+RECT
 if(-not [SingleFileUI]::GetClientRect($window,[ref]$r)){throw 'Chooser bounds unavailable'}
 $scale=[SingleFileUI]::GetDpiForWindow($window)/96.
 $point=[SingleFileUI]::Point([int]($r.R/2+165*$scale),[int](145*$scale))
 [SingleFileUI]::SendMessageW($window,0x200,[IntPtr]::Zero,$point)|Out-Null
 [SingleFileUI]::SendMessageW($window,0x201,[IntPtr]::new(1),$point)|Out-Null
 [SingleFileUI]::SendMessageW($window,0x202,[IntPtr]::Zero,$point)|Out-Null
 $deadline=[datetime]::UtcNow.AddSeconds(20);$lastClick=[datetime]::UtcNow
 do{
  $children=@(Get-CimInstance Win32_Process|Where-Object{$_.ParentProcessId -eq $p.Id -and $_.Name -eq 'external-runtime.exe'})
  if($children.Count -eq 1 -and [SingleFileUI]::RenderWindow([uint32]$children[0].ProcessId)){$child=$children[0];break}
  if($p.HasExited){throw 'Chooser exited during launch'}
  # The new chooser can receive messages before GPUI has laid out its first
  # frame. Retry Open only while no child exists; the app also guards duplicates.
  if($children.Count -eq 0 -and ([datetime]::UtcNow-$lastClick).TotalMilliseconds -ge 500){
   [SingleFileUI]::SendMessageW($window,0x200,[IntPtr]::Zero,$point)|Out-Null
   [SingleFileUI]::SendMessageW($window,0x201,[IntPtr]::new(1),$point)|Out-Null
   [SingleFileUI]::SendMessageW($window,0x202,[IntPtr]::Zero,$point)|Out-Null
   $lastClick=[datetime]::UtcNow
  }
  [Threading.Thread]::Sleep(30)
 }while([datetime]::UtcNow -lt $deadline)
 if(-not $child){throw 'Chooser did not open its embedded native menu'}
 $runtimePath=$child.ExecutablePath
 if(-not $runtimePath.StartsWith($scratch+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)){throw 'Runtime escaped the isolated temporary directory'}
 # Window creation can precede the launch handshake; the chooser intentionally
 # rejects close while that operation is busy. Retry the user close request
 # until readiness releases the guard, within a bounded UI deadline.
 $deadline=[datetime]::UtcNow.AddSeconds(10)
 do {
  [SingleFileUI]::PostMessageW($window,0x10,[IntPtr]::Zero,[IntPtr]::Zero)|Out-Null
  if($p.WaitForExit(100)){break}
 } while([datetime]::UtcNow -lt $deadline)
 if(-not $p.HasExited){throw 'Chooser failed to close with its runtime'}
 if($p.ExitCode -ne 0){throw ('Chooser exit code '+$p.ExitCode)}
 if(Get-Process -Id $child.ProcessId -ErrorAction SilentlyContinue){throw 'Runtime outlived its single-file launcher'}
 if(@(Get-ChildItem -LiteralPath $scratch -Force).Count){throw 'GUI exit retained temporary runtime files'}
 $files=@(Get-ChildItem -LiteralPath $app -Force|Select-Object -ExpandProperty Name)
 if($files.Count -ne 1 -or $files[0] -ne 'external-everything.exe'){throw 'GUI wrote files next to its executable'}
 $proof=@{launcher_pid=$p.Id;runtime_pid=$child.ProcessId;runtime_path=$runtimePath;native_menu=$true;launcher_exit_code=$p.ExitCode;child_stopped=$true;temporary_files=@();visible_files=$files;sha256=(Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant()}
 $evidence=Join-Path $directory 'evidence.json'
 [IO.File]::WriteAllText($evidence,($proof|ConvertTo-Json -Depth 5),[Text.UTF8Encoding]::new($false))
 Write-Output "EVIDENCE=$evidence"
 Write-Output 'Verified real chooser Open, embedded native menu, launcher-close child ownership, and temporary cleanup.'
}finally{
 if($p.Id -and -not $p.HasExited){$p.CloseMainWindow()|Out-Null;$p.WaitForExit(10000)|Out-Null}
 $p.Dispose();[SingleFileUI]::SetThreadDpiAwarenessContext($old)|Out-Null
}
