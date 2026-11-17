param(
    [ValidateSet('Build', 'Check', 'Test', 'Clippy')][string]$Action = 'Build',
    [ValidateSet('Debug', 'Release')][string]$Configuration = 'Release',
    [switch]$Publish,
    [string]$PayloadDirectory
)
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
if ($Publish -and $Action -ne 'Build') { throw '-Publish requires -Action Build.' }
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
function Assert-SystemRuntime([string]$Path) {
    $command = '"' + $vsdev + '" -arch=x64 -host_arch=x64 && dumpbin.exe /nologo /dependents "' + $Path + '"'
    $dependencies = & $env:ComSpec /d /s /c $command
    if ($LASTEXITCODE -ne 0) { throw "Native dependency inspection failed: $Path" }
    foreach ($line in $dependencies) {
        if ($line.Trim() -match '^(?i:VCRUNTIME|MSVCP|MSVCR[0-9]|CONCRT|UCRTBASED|LIBGCC|LIBSTDC\+\+)[^\\/]*\.dll$') {
            throw "Shared redistributable dependency remains in ${Path}: $($line.Trim())"
        }
    }
    Write-Output "Verified statically linked VC runtime: $Path"
}
function Assert-SafePath([string]$Path) {
    $absolute = [IO.Path]::GetFullPath($Path)
    $cursor = $absolute
    while ($cursor) {
        $item = $null
        try { $item = Get-Item -LiteralPath $cursor -Force -ErrorAction Stop }
        catch [System.Management.Automation.ItemNotFoundException] { }
        if ($null -ne $item -and ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw "Reparse point in asset/publication path: $cursor"
        }
        $parent = Split-Path -Parent $cursor
        if ($parent -eq $cursor) { break }
        $cursor = $parent
    }
    return $absolute
}
function Assert-IntendedPath([string]$Path, [string]$Expected) {
    $absolute = Assert-SafePath $Path
    $intended = [IO.Path]::GetFullPath($Expected)
    if (-not [string]::Equals($absolute, $intended, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Operation path differs from its intended absolute path.'
    }
    if (Test-Path -LiteralPath $absolute) {
        $resolved = [IO.Path]::GetFullPath((Resolve-Path -LiteralPath $absolute).ProviderPath)
        if (-not [string]::Equals($resolved, $intended, [StringComparison]::OrdinalIgnoreCase)) {
            throw 'Resolved operation path differs from its intended absolute path.'
        }
    }
    return $absolute
}
function Get-BytesHash([byte[]]$Bytes) {
    $sha = [Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($sha.ComputeHash($Bytes))).Replace('-', '').ToLowerInvariant() }
    finally { $sha.Dispose() }
}
function Read-BoundedFile([string]$Path, [long]$Maximum, [long]$Minimum = 0) {
    $absolute = Assert-SafePath $Path
    $item = Get-Item -LiteralPath $absolute -Force
    if ($item.PSIsContainer) { throw "Expected a regular file: $absolute" }
    $inputFile = [IO.File]::Open($absolute, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        $length = $inputFile.Length
        if ($length -lt $Minimum -or $length -gt $Maximum) { throw "File size exceeds its bounds: $absolute" }
        $bytes = [byte[]]::new([int]$length)
        $read = 0
        while ($read -lt $bytes.Length) {
            $count = $inputFile.Read($bytes, $read, $bytes.Length - $read)
            if ($count -eq 0) { throw "File changed while reading: $absolute" }
            $read += $count
        }
        if ($inputFile.Length -ne $length -or $inputFile.ReadByte() -ne -1) { throw "File changed while reading: $absolute" }
        return ,$bytes
    } finally { $inputFile.Dispose() }
}
function Read-PeAsset([string]$Path, [ValidateSet('Efi', 'Dll', 'Runtime')][string]$Kind) {
    $data = Read-BoundedFile $Path 64MB 64
    if ([BitConverter]::ToUInt16($data, 0) -ne 0x5A4D) { throw "Invalid DOS header: $Path" }
    $offset = [long][BitConverter]::ToUInt32($data, 60)
    if ($offset -lt 64 -or $offset -gt ($data.Length - 136) -or [BitConverter]::ToUInt32($data, [int]$offset) -ne 0x4550) { throw "Invalid PE signature: $Path" }
    $optionalLength = [BitConverter]::ToUInt16($data, [int]$offset + 20)
    if ($optionalLength -lt 112 -or ($offset + 24 + $optionalLength) -gt $data.Length) { throw "Truncated PE header: $Path" }
    if ([BitConverter]::ToUInt16($data, [int]$offset + 4) -ne 0x8664 -or [BitConverter]::ToUInt16($data, [int]$offset + 24) -ne 0x20B) { throw "Expected x64 PE32+ image: $Path" }
    $flags = [BitConverter]::ToUInt16($data, [int]$offset + 22)
    if (($flags -band 0x0002) -eq 0) { throw "PE executable-image flag is missing: $Path" }
    if ($Kind -eq 'Efi' -and [BitConverter]::ToUInt16($data, [int]$offset + 92) -ne 10) { throw 'Standalone EFI_APPLICATION is required.' }
    if ($Kind -eq 'Dll' -and ($flags -band 0x2000) -eq 0) { throw 'Attachment DLL flag is missing.' }
    if ($Kind -eq 'Runtime' -and ($flags -band 0x2000) -ne 0) { throw 'External runtime must be an executable, rather than a DLL.' }
    return [pscustomobject]@{ Bytes = $data; Hash = Get-BytesHash $data }
}
function Assert-Fields($Object, [string[]]$Required, [string[]]$Optional = @()) {
    if ($Object -isnot [pscustomobject]) { throw 'Expected a JSON object.' }
    $names = @($Object.PSObject.Properties.Name)
    foreach ($name in $Required) {
        if ($names -cnotcontains $name) { throw "Required catalog field is absent: $name" }
    }
    foreach ($name in $names) {
        if (($Required -cnotcontains $name) -and ($Optional -cnotcontains $name)) { throw "Unexpected catalog field: $name" }
    }
}
function Read-PointBlankCatalog([byte[]]$Bytes, [switch]$Legacy) {
    $text = [Text.UTF8Encoding]::new($false, $true).GetString($Bytes)
    $catalog = $text | ConvertFrom-Json -ErrorAction Stop
    Assert-Fields $catalog @('schema', 'games')
    if (($catalog.schema -isnot [int] -and $catalog.schema -isnot [long]) -or $catalog.schema -ne 1 -or $catalog.games -isnot [array] -or $catalog.games.Count -ne 1) { throw 'Expected schema 1 and exactly one registered PointBlank game.' }
    $entry = $catalog.games[0]
    if ($Legacy) { Assert-Fields $entry @('id', 'name', 'executable', 'sha256') }
    else { Assert-Fields $entry @('id', 'name') @('executable', 'sha256') }
    if ($entry.id -isnot [string] -or $entry.id -cne 'pointblank' -or $entry.name -isnot [string] -or $entry.name -cne 'PointBlank') { throw 'PointBlank catalog identity differs.' }
    $hasExecutable = @($entry.PSObject.Properties.Name) -ccontains 'executable'
    $hasHash = @($entry.PSObject.Properties.Name) -ccontains 'sha256'
    if ($hasExecutable -ne $hasHash) { throw 'Catalog executable and SHA-256 require each other.' }
    if ($hasExecutable -and ($entry.executable -isnot [string] -or $entry.executable -cne 'pointblank/external-runtime.exe' -or $entry.sha256 -isnot [string] -or $entry.sha256 -cnotmatch '^[a-f0-9]{64}$')) { throw 'PointBlank runtime path or SHA-256 differs.' }
    return $entry
}
function Write-FreshFile([byte[]]$Bytes, [string]$Directory, [string]$Name) {
    $directoryPath = Assert-SafePath $Directory
    $destination = [IO.Path]::GetFullPath((Join-Path $directoryPath $Name))
    if ([IO.Path]::GetDirectoryName($destination) -ne $directoryPath) { throw 'Fresh file escapes its intended directory.' }
    $destination = Assert-IntendedPath $destination (Join-Path $directoryPath $Name)
    $output = [IO.File]::Open($destination, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try { $output.Write($Bytes, 0, $Bytes.Length); $output.Flush($true) } finally { $output.Dispose() }
    if ((Get-BytesHash $Bytes) -cne (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant()) { throw 'Fresh asset hash differs.' }
    return $destination
}
function Publish-Fresh([string]$Source, [string]$Directory, [string]$Name) {
    $directoryPath = Assert-SafePath $Directory
    New-Item -ItemType Directory -Force -Path $directoryPath | Out-Null
    $directoryPath = Assert-IntendedPath (Resolve-Path -LiteralPath $directoryPath).ProviderPath $directoryPath
    $destination = [IO.Path]::GetFullPath((Join-Path $directoryPath $Name))
    if ([IO.Path]::GetDirectoryName($destination) -ne $directoryPath) { throw 'Publish file escapes its intended directory.' }
    $destination = Assert-IntendedPath $destination (Join-Path $directoryPath $Name)
    if ((Test-Path -LiteralPath $destination) -and (Get-Item -LiteralPath $destination -Force).PSIsContainer) { throw 'Publish destination is a directory.' }
    $stageName = '.publish-' + [guid]::NewGuid().ToString('N') + '.tmp'
    $bytes = Read-BoundedFile $Source 256MB 64
    $hash = Get-BytesHash $bytes
    $stage = Write-FreshFile $bytes $directoryPath $stageName
    $stage = Assert-IntendedPath $stage (Join-Path $directoryPath $stageName)
    $destination = Assert-IntendedPath $destination (Join-Path $directoryPath $Name)
    Write-Output "Verified rename source: $stage"
    Write-Output "Verified rename destination: $destination"
    Move-Item -LiteralPath $stage -Destination $destination -Force
    if ($hash -cne (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant()) { throw 'Published hash differs.' }
    Write-Output "Published $Name SHA256 $hash"
}
function Remove-VerifiedLegacyFile([string]$Directory, [string]$Relative, [string]$ExpectedHash) {
    $expected = [IO.Path]::GetFullPath((Join-Path $Directory $Relative))
    try {
        $path = Assert-IntendedPath $expected $expected
        if (-not (Test-Path -LiteralPath $path)) { return $false }
        $bytes = Read-BoundedFile $path 64MB
        if ((Get-BytesHash $bytes) -cne $ExpectedHash) { throw 'Legacy identity differs.' }
        # Recheck identity and every ancestor immediately before the bounded file deletion.
        $path = Assert-IntendedPath $path $expected
        if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() -cne $ExpectedHash) { throw 'Legacy file changed before cleanup.' }
        Write-Output "Verified legacy removal: $path" | Out-Host
        Remove-Item -LiteralPath $path -Force
        return $true
    } catch {
        Write-Warning "Preserved legacy path $expected : $($_.Exception.Message)"
        return $false
    }
}
function Remove-EmptyKnownDirectory([string]$Directory, [string]$Relative) {
    $expected = [IO.Path]::GetFullPath((Join-Path $Directory $Relative))
    try {
        $path = Assert-IntendedPath $expected $expected
        if (-not (Test-Path -LiteralPath $path)) { return }
        if (-not (Get-Item -LiteralPath $path -Force).PSIsContainer) { throw 'Expected a directory.' }
        if (@(Get-ChildItem -LiteralPath $path -Force).Count -ne 0) { throw 'Directory still contains preserved data.' }
        $path = Assert-IntendedPath $path $expected
        Write-Output "Verified empty legacy directory removal: $path"
        # Directory.Delete(false) is deliberately nonrecursive and rechecks emptiness.
        [IO.Directory]::Delete($path, $false)
    } catch { Write-Warning "Preserved legacy directory $expected : $($_.Exception.Message)" }
}
function Remove-LegacySidecars([string]$Directory, [string]$EfiHash, [string]$DllHash) {
    $directoryPath = Assert-SafePath $Directory
    $null = Remove-VerifiedLegacyFile $directoryPath 'uefi-boot-standalone.efi' $EfiHash
    $null = Remove-VerifiedLegacyFile $directoryPath 'hyperv-attachment.dll' $DllHash
    $catalogPath = [IO.Path]::GetFullPath((Join-Path $directoryPath 'games\catalog.json'))
    try {
        $catalogPath = Assert-IntendedPath $catalogPath (Join-Path $directoryPath 'games\catalog.json')
        if (Test-Path -LiteralPath $catalogPath) {
            $catalogBytes = Read-BoundedFile $catalogPath 1MB 1
            $legacy = Read-PointBlankCatalog $catalogBytes -Legacy
            $catalogHash = Get-BytesHash $catalogBytes
            $runtimePath = Assert-IntendedPath (Join-Path $directoryPath 'games\pointblank\external-runtime.exe') (Join-Path $directoryPath 'games\pointblank\external-runtime.exe')
            $legacyRuntime = Read-PeAsset $runtimePath 'Runtime'
            if ($legacyRuntime.Hash -cne $legacy.sha256) { throw 'Legacy runtime differs from its registered SHA-256.' }
            if ((Get-FileHash -LiteralPath $catalogPath -Algorithm SHA256).Hash.ToLowerInvariant() -cne $catalogHash) { throw 'Legacy catalog changed before cleanup.' }
            if (Remove-VerifiedLegacyFile $directoryPath 'games\pointblank\external-runtime.exe' $legacy.sha256) {
                $null = Remove-VerifiedLegacyFile $directoryPath 'games\catalog.json' $catalogHash
            }
        }
    } catch { Write-Warning "Preserved legacy game data at $catalogPath : $($_.Exception.Message)" }
    Remove-EmptyKnownDirectory $directoryPath 'games\pointblank'
    Remove-EmptyKnownDirectory $directoryPath 'games'
    foreach ($item in @(Get-ChildItem -LiteralPath $directoryPath -Force)) {
        if ($item.Name -cne 'external-everything.exe') { Write-Warning "Preserved additional existing data: $($item.FullName)" }
    }
}

# An inherited bundle path must never affect dev checks, tests, Clippy, or runtime compilation.
$bundleVariable = 'EXTERNAL_EVERYTHING_BUNDLE_DIR'
$hadBundleVariable = Test-Path "Env:\$bundleVariable"
$previousBundleDirectory = [Environment]::GetEnvironmentVariable($bundleVariable, 'Process')
$target = [IO.Path]::GetFullPath($env:CARGO_TARGET_DIR)
$profile = $Configuration.ToLowerInvariant()
$nativeTarget = 'x86_64-pc-windows-msvc'
$artifactRoot = if ($Action -eq 'Build') { Join-Path $target $nativeTarget } else { $target }
$runtime = Join-Path $artifactRoot ($profile + '\external-runtime.exe')
$launcher = Join-Path $artifactRoot ($profile + '\external-everything.exe')
$hadRustFlags = Test-Path Env:RUSTFLAGS
$previousRustFlags = $env:RUSTFLAGS
$hadEncodedFlags = Test-Path Env:CARGO_ENCODED_RUSTFLAGS
$previousEncodedFlags = $env:CARGO_ENCODED_RUSTFLAGS
try {
    Remove-Item "Env:\$bundleVariable" -ErrorAction SilentlyContinue
    if ($Action -eq 'Build') {
        # An explicit target keeps target CRT flags away from host proc macros.
        # Statically link the VC runtime so the distributable needs only Windows DLLs.
        if ($hadEncodedFlags) {
            $separator=[string][char]31
            $env:CARGO_ENCODED_RUSTFLAGS=$previousEncodedFlags+$separator+'-C'+$separator+'target-feature=+crt-static'
        } else {
            $env:RUSTFLAGS=($previousRustFlags+' -C target-feature=+crt-static').Trim()
        }
        $runtimeArguments = @('build', '-p', 'external-runtime', '--target', $nativeTarget, '--locked', '--manifest-path', ('"' + $manifest + '"'))
        if ($Configuration -eq 'Release') { $runtimeArguments += '--release' }
        Invoke-Cargo $runtimeArguments

        # Payloads are prebuilt from the pinned hyper-reV fork; this helper never rebuilds or updates that source.
        if (-not $PayloadDirectory) { $PayloadDirectory = Join-Path $vendor 'build\Release' }
        $payloadPath = Assert-SafePath $PayloadDirectory
        $PayloadDirectory = (Resolve-Path -LiteralPath $payloadPath).ProviderPath
        if (-not (Get-Item -LiteralPath $PayloadDirectory -Force).PSIsContainer) { throw 'PayloadDirectory must be an existing directory.' }
        $efi = Read-PeAsset (Join-Path $PayloadDirectory 'uefi-boot-standalone.efi') 'Efi'
        $dll = Read-PeAsset (Join-Path $PayloadDirectory 'hyperv-attachment.dll') 'Dll'
        $runtimeAsset = Read-PeAsset $runtime 'Runtime'
        $sourceCatalog = Read-PointBlankCatalog (Read-BoundedFile (Join-Path $root 'games\catalog.json') 1MB 1)
        $catalog = [ordered]@{
            schema = 1
            games = @([ordered]@{
                id = $sourceCatalog.id
                name = $sourceCatalog.name
                executable = 'pointblank/external-runtime.exe'
                sha256 = $runtimeAsset.Hash
            })
        }
        $catalogBytes = [Text.UTF8Encoding]::new($false).GetBytes(($catalog | ConvertTo-Json -Depth 8) + "`n")
        $null = Read-PointBlankCatalog $catalogBytes -Legacy
        $buildDirectory = Assert-SafePath (Join-Path $root 'build')
        $embedDirectory = Assert-SafePath (Join-Path $buildDirectory ('embed-' + [guid]::NewGuid().ToString('N')))
        New-Item -ItemType Directory -Path $embedDirectory | Out-Null
        $embedDirectory = Assert-IntendedPath (Resolve-Path -LiteralPath $embedDirectory).ProviderPath $embedDirectory
        $gamesDirectory = Join-Path $embedDirectory 'games'
        $pointblankDirectory = Join-Path $gamesDirectory 'pointblank'
        New-Item -ItemType Directory -Path $gamesDirectory | Out-Null
        New-Item -ItemType Directory -Path $pointblankDirectory | Out-Null
        $null = Write-FreshFile $efi.Bytes $embedDirectory 'uefi-boot-standalone.efi'
        $null = Write-FreshFile $dll.Bytes $embedDirectory 'hyperv-attachment.dll'
        $null = Write-FreshFile $catalogBytes $gamesDirectory 'catalog.json'
        $null = Write-FreshFile $runtimeAsset.Bytes $pointblankDirectory 'external-runtime.exe'
        Write-Output "Embedding fresh private payload directory: $embedDirectory"
        $env:EXTERNAL_EVERYTHING_BUNDLE_DIR = $embedDirectory
    }
    $arguments = @($Action.ToLowerInvariant(), '--manifest-path', ('"' + $manifest + '"'), '--workspace', '--locked')
    if ($Action -eq 'Build') { $arguments += @('--target', $nativeTarget) }
    if ($Action -in @('Build', 'Test') -and $Configuration -eq 'Release') { $arguments += '--release' }
    if ($Action -eq 'Clippy') { $arguments += @('--all-targets', '--', '-D', 'warnings') }
    Invoke-Cargo $arguments
    if ($Action -eq 'Test') {
        $vendorArguments = @('test', '--manifest-path', ('"' + (Join-Path $vendor 'loader\Cargo.toml') + '"'), '--locked')
        if ($Configuration -eq 'Release') { $vendorArguments += '--release' }
        Invoke-Cargo $vendorArguments
    }
} finally {
    if ($hadRustFlags) { $env:RUSTFLAGS=$previousRustFlags } else { Remove-Item Env:RUSTFLAGS -ErrorAction SilentlyContinue }
    if ($hadEncodedFlags) { $env:CARGO_ENCODED_RUSTFLAGS=$previousEncodedFlags } else { Remove-Item Env:CARGO_ENCODED_RUSTFLAGS -ErrorAction SilentlyContinue }
    if ($hadBundleVariable) { [Environment]::SetEnvironmentVariable($bundleVariable, $previousBundleDirectory, 'Process') }
    else { Remove-Item "Env:\$bundleVariable" -ErrorAction SilentlyContinue }
}
if ($Action -ne 'Build') { return }
if (-not (Test-Path -LiteralPath $launcher -PathType Leaf)) { throw 'Built launcher is absent.' }
Assert-SystemRuntime $runtime
Assert-SystemRuntime $launcher
$destinations = @((Join-Path $root 'dist'))
if ($Publish) {
    if (-not $env:LOCALAPPDATA) { throw 'LOCALAPPDATA is absent.' }
    $destinations += Join-Path $env:LOCALAPPDATA 'External Everything\bin'
}
foreach ($directory in $destinations) {
    # Distribution contains just the executable. Intermediate embed inputs stay under build/.
    Publish-Fresh $launcher $directory 'external-everything.exe'
    Remove-LegacySidecars $directory $efi.Hash $dll.Hash
}
