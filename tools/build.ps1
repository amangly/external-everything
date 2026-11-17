param(
    [ValidateSet('Build', 'Check', 'Test', 'Clippy')][string]$Action = 'Build',
    [ValidateSet('Debug', 'Release')][string]$Configuration = 'Release',
    [switch]$Publish,
    [string]$PayloadDirectory
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
if (-not $env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = Join-Path $root 'target' }
$manifest = Join-Path $root 'Cargo.toml'
$vendor = Join-Path $root 'vendor\hyper-reV'
if (-not (Test-Path -LiteralPath (Join-Path $vendor 'loader\Cargo.toml'))) { throw 'Initialize vendor first: git submodule update --init --recursive' }
$cargo = (Get-Command cargo.exe -ErrorAction Stop).Source
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (-not (Test-Path -LiteralPath $vswhere)) { throw 'Install Visual Studio C++ build tools and a Windows SDK.' }
$installation = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if ($LASTEXITCODE -ne 0 -or -not $installation) { throw 'Visual Studio x64 C++ tools are missing.' }
$vsdev = Join-Path $installation 'Common7\Tools\VsDevCmd.bat'
$temp = Join-Path $root 'build\temp'
New-Item -ItemType Directory -Force -Path $temp | Out-Null
$env:TEMP = $temp
$env:TMP = $temp
if (-not $env:GPUI_FXC_PATH) {
    $sdk = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
    $fxc = Get-ChildItem -Path (Join-Path $sdk '*\x64\fxc.exe') -File -ErrorAction SilentlyContinue | Sort-Object FullName -Descending | Select-Object -First 1
    if ($fxc) { $env:GPUI_FXC_PATH = $fxc.FullName }
}
function Invoke-Cargo([string[]]$Arguments) {
    $command = '"' + $vsdev + '" -arch=x64 -host_arch=x64 && "' + $cargo + '" ' + ($Arguments -join ' ')
    & $env:ComSpec /d /s /c $command
    if ($LASTEXITCODE -ne 0) { throw "Cargo failed with exit code $LASTEXITCODE" }
}
$arguments = @($Action.ToLowerInvariant(), '--manifest-path', ('"' + $manifest + '"'), '--workspace', '--locked')
if ($Action -in @('Build', 'Test') -and $Configuration -eq 'Release') { $arguments += '--release' }
if ($Action -eq 'Clippy') { $arguments += @('--all-targets', '--', '-D', 'warnings') }
Invoke-Cargo $arguments
if ($Action -eq 'Test') {
    $vendorArguments = @('test', '--manifest-path', ('"' + (Join-Path $vendor 'loader\Cargo.toml') + '"'), '--locked')
    if ($Configuration -eq 'Release') { $vendorArguments += '--release' }
    Invoke-Cargo $vendorArguments
}
if ($Action -ne 'Build') {
    if ($Publish) { throw '-Publish requires -Action Build.' }
    return
}
if (-not $PayloadDirectory) { $PayloadDirectory = Join-Path $vendor 'build\Release' }
$PayloadDirectory = (Resolve-Path -LiteralPath $PayloadDirectory).Path
$target = if ($env:CARGO_TARGET_DIR) { [IO.Path]::GetFullPath($env:CARGO_TARGET_DIR) } else { Join-Path $root 'target' }
$sources = [ordered]@{ 'external-everything.exe' = Join-Path $target ($Configuration.ToLowerInvariant() + '\external-everything.exe') }
foreach ($name in @('uefi-boot-standalone.efi', 'hyperv-attachment.dll')) {
    $source = Join-Path $PayloadDirectory $name
    $data = [IO.File]::ReadAllBytes($source)
    if ($data.Length -lt 64 -or $data.Length -gt 64MB -or [BitConverter]::ToUInt16($data,0) -ne 0x5A4D) { throw "Invalid PE image: $name" }
    $offset = [BitConverter]::ToUInt32($data,60)
    if ($offset -gt ($data.Length-136) -or [BitConverter]::ToUInt32($data,$offset) -ne 0x4550) { throw "Invalid PE signature: $name" }
    $optionalLength = [BitConverter]::ToUInt16($data,$offset+20)
    if ($optionalLength -lt 112 -or ([long]$offset+24+$optionalLength) -gt $data.Length) { throw "Truncated PE header: $name" }
    if ([BitConverter]::ToUInt16($data,$offset+4) -ne 0x8664 -or [BitConverter]::ToUInt16($data,$offset+24) -ne 0x20B) { throw "Expected x64 PE32+ image: $name" }
    if ($name.EndsWith('.efi') -and [BitConverter]::ToUInt16($data,$offset+92) -ne 10) { throw 'Standalone EFI_APPLICATION is required.' }
    if ($name.EndsWith('.dll') -and ([BitConverter]::ToUInt16($data,$offset+22) -band 0x2000) -eq 0) { throw 'Attachment DLL flag is missing.' }
    $sources[$name] = $source
}
function Publish-Fresh([string]$Source, [string]$Directory, [string]$Name) {
    New-Item -ItemType Directory -Force -Path $Directory | Out-Null
    $directoryPath = (Resolve-Path -LiteralPath $Directory).Path
    if ((Get-Item -LiteralPath $directoryPath).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Publish directory is a reparse point.' }
    $destination = [IO.Path]::GetFullPath((Join-Path $directoryPath $Name))
    if (Test-Path -LiteralPath $destination) {
        $existing = Get-Item -LiteralPath $destination
        if ($existing.PSIsContainer -or ($existing.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Publish destination is a directory or reparse point.' }
    }
    $stage = [IO.Path]::GetFullPath((Join-Path $directoryPath ('.publish-' + [guid]::NewGuid().ToString('N') + '.tmp')))
    if ([IO.Path]::GetDirectoryName($stage) -ne $directoryPath -or [IO.Path]::GetDirectoryName($destination) -ne $directoryPath) { throw 'Publish path differs from the intended directory.' }
    $bytes = [IO.File]::ReadAllBytes($Source)
    $output = [IO.File]::Open($stage,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
    try { $output.Write($bytes,0,$bytes.Length); $output.Flush($true) } finally { $output.Dispose() }
    $hash = (Get-FileHash -LiteralPath $Source -Algorithm SHA256).Hash
    if ($hash -ne (Get-FileHash -LiteralPath $stage -Algorithm SHA256).Hash) { throw 'Staged publish hash differs.' }
    Write-Output "Verified rename source: $stage"
    Write-Output "Verified rename destination: $destination"
    Move-Item -LiteralPath $stage -Destination $destination -Force
    if ($hash -ne (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash) { throw 'Published hash differs.' }
    Write-Output "Published $Name SHA256 $hash"
}
$destinations = @((Join-Path $root 'dist'))
if ($Publish) { $destinations += Join-Path $env:LOCALAPPDATA 'External Everything\bin' }
foreach ($directory in $destinations) {
    foreach ($name in $sources.Keys) { Publish-Fresh $sources[$name] $directory $name }
    Publish-Fresh (Join-Path $root 'games\catalog.json') (Join-Path $directory 'games') 'catalog.json'
}
