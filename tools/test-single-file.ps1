param([string]$Executable=(Join-Path (Split-Path -Parent $PSScriptRoot) 'dist\external-everything.exe'))
$ErrorActionPreference='Stop'
$source=(Resolve-Path -LiteralPath $Executable).Path
$root=Split-Path -Parent $PSScriptRoot
$directory=Join-Path $root ('build\single-file-verification-'+[guid]::NewGuid().ToString('N'))
$app=Join-Path $directory 'app';$scratch=Join-Path $directory 'temp';$local=Join-Path $directory 'local'
New-Item -ItemType Directory -Path $app,$scratch,$local|Out-Null
$exe=Join-Path $app 'external-everything.exe'
Copy-Item -LiteralPath $source -Destination $exe
if((Get-FileHash -LiteralPath $source).Hash -ne (Get-FileHash -LiteralPath $exe).Hash){throw 'Copied single executable differs'}
function Invoke-Single([string]$Arguments,[int]$Timeout=45000){
 $s=New-Object Diagnostics.ProcessStartInfo
 $s.FileName=$exe;$s.Arguments=$Arguments;$s.WorkingDirectory=$app;$s.UseShellExecute=$false;$s.CreateNoWindow=$true;$s.RedirectStandardOutput=$true;$s.RedirectStandardError=$true
 $s.EnvironmentVariables['TEMP']=$scratch;$s.EnvironmentVariables['TMP']=$scratch;$s.EnvironmentVariables['LOCALAPPDATA']=$local
 $p=New-Object Diagnostics.Process;$p.StartInfo=$s
 try{
  if(-not $p.Start()){throw 'Single-file check failed to start'}
  $handle=$p.Handle;$o=$p.StandardOutput.ReadToEndAsync();$e=$p.StandardError.ReadToEndAsync()
  if(-not $p.WaitForExit($Timeout)){$p.Kill();$p.WaitForExit();throw ('Single-file check deadline: '+$Arguments)}
  if($p.ExitCode -ne 0){throw ($Arguments+': '+$e.Result)}
  $result=$o.Result|ConvertFrom-Json
  if(@(Get-ChildItem -LiteralPath $scratch -Force).Count){throw ('Temporary files remain after '+$Arguments)}
  $files=@(Get-ChildItem -LiteralPath $app -Force)
  if($files.Count -ne 1 -or $files[0].Name -ne 'external-everything.exe'){throw 'The executable wrote sidecar files into its folder'}
  return $result
 }finally{$p.Dispose()}
}
$bundle=Invoke-Single '--inspect-bundle'
if(-not $bundle.single_file -or $bundle.assets.Count -ne 4 -or @($bundle.assets|Where-Object{-not $_.verified}).Count){throw 'Embedded asset metadata is incomplete'}
$staging=Invoke-Single '--verify-bundle'
if(-not $staging.boot_staging_cleaned -or -not $staging.game_staging_cleaned -or $staging.boot_mutations){throw 'Boot/game staging verification differs'}
$flow=Invoke-Single '--inspect-flow'
if($flow.games -ne 1 -or -not $flow.single_file){throw 'Embedded game chooser registration differs'}
$contract=Invoke-Single '--runtime contract --json'
if($contract.backend_abi -ne 'hyper-rev/cpuid-8a3b55c7927/v1'){throw 'Embedded runtime ABI differs'}
$launch=$null;$render=$null;$memory=$null;$snapshot=$null
if($flow.stage -eq 'games'){
 $memory=Invoke-Single '--runtime --self-test'
 if(-not $memory.attachment -or $memory.memory_read_write -ne 'verified'){throw 'Embedded hypercall transport check failed'}
 $snapshot=Invoke-Single '--runtime --snapshot'
 $render=Invoke-Single '--runtime --render-smoke 3'
 if(-not $render.d2d_ready -or -not $render.d3d11_ready -or $render.reader_hz -lt 30){throw 'Embedded native renderers or reader cadence failed'}
 $launch=Invoke-Single '--verify-runtime-launch'
 if(-not $launch.started -or -not $launch.window_ready -or -not $launch.staging_cleaned){throw 'Single-file launcher handoff/cleanup failed'}
 if(Get-Process -Id $launch.runtime_pid -ErrorAction SilentlyContinue){throw 'Single-file launcher child survived cleanup'}
}
$proof=@{executable=$exe;sha256=(Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant();bundle=$bundle;staging=$staging;flow=$flow;contract=$contract;transport=$memory;snapshot=$snapshot;render=$render;launch=$launch;visible_files=@(Get-ChildItem -LiteralPath $app -Force|Select-Object -ExpandProperty Name);temporary_files=@(Get-ChildItem -LiteralPath $scratch -Force|Select-Object -ExpandProperty Name);boot_mutations=$false}
$evidence=Join-Path $directory 'evidence.json'
[IO.File]::WriteAllText($evidence,($proof|ConvertTo-Json -Depth 15),[Text.UTF8Encoding]::new($false))
Write-Output "EVIDENCE=$evidence"
Write-Output "CLEAN_EXE=$exe"
Write-Output 'Verified embedded boot sources, catalog, native runtime and staging cleanup with one visible executable.'
if($launch){Write-Output ($launch|ConvertTo-Json -Compress)}
