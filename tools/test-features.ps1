param([string]$Launcher=(Join-Path $env:LOCALAPPDATA 'External Everything\bin\external-everything.exe'))
$ErrorActionPreference='Stop'
$launcherPath=(Resolve-Path -LiteralPath $Launcher).Path
$runtime=Join-Path (Split-Path -Parent $launcherPath) 'games\pointblank\external-runtime.exe'
$root=Split-Path -Parent $PSScriptRoot
$directory=Join-Path $root ('build\feature-verification-'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $directory|Out-Null
function Invoke-Json([string]$Arguments){
 $s=New-Object Diagnostics.ProcessStartInfo
 $s.FileName=$runtime;$s.Arguments=$Arguments;$s.UseShellExecute=$false;$s.CreateNoWindow=$true;$s.RedirectStandardOutput=$true;$s.RedirectStandardError=$true
 $p=New-Object Diagnostics.Process;$p.StartInfo=$s
 try {
  if(-not $p.Start()){throw 'Feature verification child start failed'}
  $handle=$p.Handle;$o=$p.StandardOutput.ReadToEndAsync();$e=$p.StandardError.ReadToEndAsync()
  if(-not $p.WaitForExit(15000)){$p.Kill();$p.WaitForExit();throw 'Feature verification child exceeded deadline'}
  if($p.ExitCode -ne 0){throw $e.Result}
  return $o.Result|ConvertFrom-Json
 }finally{$p.Dispose()}
}
$snapshot=Invoke-Json '--snapshot'
if(-not $snapshot.pid -or -not $snapshot.features.available){throw 'Feature verification requires the supported running PointBlank build'}
$targetPid=[uint32]$snapshot.pid
$request=Join-Path $directory 'request.json'
$ranges=@(@{label='names';address=[uint64]($snapshot.base+0x67922c);size=21},@{label='anti_kick';address=[uint64]($snapshot.base+0x73d4d6);size=39})
[IO.File]::WriteAllText($request,(ConvertTo-Json -InputObject $ranges -Depth 4),[Text.UTF8Encoding]::new($false))
$expected=@('6a1281c13d06000058c601008d490883e80175f5c3','558bec8b4508568bf185c078150f9fc00fb6c050e83ad9ffff6a088bcee816dfdaff5e5dc20800')
function Read-Guards([string]$Name,[bool]$Enabled){
 $deadline=[datetime]::UtcNow.AddSeconds(5);$attempt=0
 do {
  $output=Join-Path $directory ($Name+'-'+$attempt+'.json');$attempt++
  $null=Invoke-Json ('--read-pointblank '+$targetPid+' "'+$request+'" "'+$output+'"')
  $data=[IO.File]::ReadAllText($output)|ConvertFrom-Json
  if($data.pid -ne $targetPid -or $data.image_base -ne $snapshot.base){throw 'Feature verification target identity changed'}
  $complete=$true
  for($i=0;$i -lt 2;$i++){
   $want=$expected[$i]
   if($Enabled){$byte=if($i -eq 0){'01'}else{'eb'};$want=$want.Substring(0,22)+$byte+$want.Substring(24)}
   if($data.reads[$i].hex -ne $want){
    if($Enabled -and $data.reads[$i].hex -eq $expected[$i]){$complete=$false}
    else{throw ('Full function guard differs: '+$data.reads[$i].label+' in '+$Name)}
   }
  }
  if($complete){return $output}
  [Threading.Thread]::Sleep(10)
 }while([datetime]::UtcNow -lt $deadline)
 throw 'Renderer reached presentation readiness but its feature patches never became active'
}
$before=Read-Guards 'before' $false
$roundtrip=Invoke-Json '--verify-features'
if($roundtrip.names_patch -ne 'verified' -or $roundtrip.anti_kick_patch -ne 'verified' -or $roundtrip.original_bytes -ne 'restored'){throw 'Native feature roundtrip differs'}
$afterRoundtrip=Read-Guards 'after-roundtrip' $false
# Isolate this test's feature configuration from the user's saved settings.
$settingsDirectory=Join-Path $directory 'local\External Everything\pointblank'
New-Item -ItemType Directory -Path $settingsDirectory -Force|Out-Null
$config=@{names=$true;anti_kick=$true;macro_enabled=$false;crosshair=$true;crosshair_style='dot';theme=1}
[IO.File]::WriteAllText((Join-Path $settingsDirectory 'settings.json'),($config|ConvertTo-Json),[Text.UTF8Encoding]::new($false))
Add-Type -TypeDefinition @'
using System;using System.Text;using System.Runtime.InteropServices;
public static class FeatureMenuProbe{
 public delegate bool Callback(IntPtr h,IntPtr p);
 [DllImport("user32.dll")]public static extern bool EnumWindows(Callback cb,IntPtr p);
 [DllImport("user32.dll")]public static extern uint GetWindowThreadProcessId(IntPtr h,out uint pid);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)]public static extern int GetClassNameW(IntPtr h,StringBuilder s,int n);
 [DllImport("user32.dll")]public static extern bool PostMessageW(IntPtr h,uint m,IntPtr w,IntPtr l);
 public static IntPtr Find(uint pid){IntPtr found=IntPtr.Zero;EnumWindows((h,p)=>{uint owner;GetWindowThreadProcessId(h,out owner);if(owner==pid){var s=new StringBuilder(256);GetClassNameW(h,s,256);if(s.ToString().IndexOf("Imgui",StringComparison.OrdinalIgnoreCase)>=0){found=h;return false;}}return true;},IntPtr.Zero);return found;}
}
'@
$eventName='Local\ExternalEverything-runtime-feature-ready-'+[guid]::NewGuid().ToString('N')
$created=$false;$ready=[Threading.EventWaitHandle]::new($false,[Threading.EventResetMode]::ManualReset,$eventName,[ref]$created)
$s=New-Object Diagnostics.ProcessStartInfo
$s.FileName=$runtime;$s.Arguments='--render-smoke 8';$s.UseShellExecute=$false;$s.CreateNoWindow=$true;$s.RedirectStandardOutput=$true;$s.RedirectStandardError=$true
$s.EnvironmentVariables['LOCALAPPDATA']=Join-Path $directory 'local'
$s.EnvironmentVariables['EXTERNAL_EVERYTHING_READY_EVENT']=$eventName
$child=New-Object Diagnostics.Process;$child.StartInfo=$s
try {
 if(-not $child.Start()){throw 'Feature-enabled renderer start failed'}
 $handle=$child.Handle;$o=$child.StandardOutput.ReadToEndAsync();$e=$child.StandardError.ReadToEndAsync()
 if(-not $ready.WaitOne(10000)){
  $reason=if($child.HasExited){$e.Result}else{'Child is still running'}
  throw ('Feature-enabled renderer missed its ready deadline: '+$reason)
 }
 $during=Read-Guards 'while-rendering' $true
 $menu=[FeatureMenuProbe]::Find([uint32]$child.Id)
 if($menu -eq [IntPtr]::Zero){throw 'Feature-enabled menu window is absent'}
 if(-not [FeatureMenuProbe]::PostMessageW($menu,0x10,[IntPtr]::Zero,[IntPtr]::Zero)){throw 'Feature-enabled menu close failed'}
 if(-not $child.WaitForExit(10000)){throw 'Feature-enabled renderer failed to shut down'}
 if($child.ExitCode -ne 0){throw $e.Result}
 $render=$o.Result|ConvertFrom-Json
 if(-not $render.d2d_ready -or -not $render.d3d11_ready){throw 'Feature-enabled native renderers are incomplete'}
 $afterClose=Read-Guards 'after-window-close' $false
 $proof=@{pid=$targetPid;roundtrip=$roundtrip;before=$before;after_roundtrip=$afterRoundtrip;while_rendering=$during;after_window_close=$afterClose;render=$render;macro_input_enabled=$false;original_bytes='restored';runtime_sha256=(Get-FileHash -LiteralPath $runtime -Algorithm SHA256).Hash.ToLowerInvariant()}
 $output=Join-Path $directory 'evidence.json'
 [IO.File]::WriteAllText($output,($proof|ConvertTo-Json -Depth 8),[Text.UTF8Encoding]::new($false))
 Write-Output "EVIDENCE=$output"
 Write-Output ($roundtrip|ConvertTo-Json -Compress)
 Write-Output 'Verified independent and combined feature writes, full-function readbacks, and native-window-close restoration.'
}finally{
 if($child.Id -and -not $child.HasExited){
  $menu=[FeatureMenuProbe]::Find([uint32]$child.Id)
  if($menu -ne [IntPtr]::Zero){[FeatureMenuProbe]::PostMessageW($menu,0x10,[IntPtr]::Zero,[IntPtr]::Zero)|Out-Null}
  if(-not $child.WaitForExit(5000)){$child.Kill();$child.WaitForExit()}
 }
 $child.Dispose();$ready.Dispose()
}
