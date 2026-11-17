param([string]$Ref = 'main', [switch]$Recurse)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$vendor = Join-Path $root 'vendor\hyper-reV'
if ($Ref -notmatch '^[A-Za-z0-9][A-Za-z0-9._/-]{0,199}$' -or $Ref.Contains('..')) { throw 'Use a branch, tag, or commit SHA.' }
& git -C $root submodule update --init -- vendor/hyper-reV
if ($LASTEXITCODE -ne 0) { throw 'Vendor initialization failed.' }
$actual = & git -C $vendor rev-parse --show-toplevel
if ($LASTEXITCODE -ne 0 -or [IO.Path]::GetFullPath($actual) -ne [IO.Path]::GetFullPath($vendor)) { throw 'Vendor checkout path differs.' }
$remote = & git -C $vendor remote get-url origin
if ($LASTEXITCODE -ne 0 -or $remote -ne 'https://github.com/amangly/hyper-reV.git') { throw 'Vendor origin differs from the configured fork.' }
$dirty = & git -C $vendor status --porcelain --untracked-files=no
if ($LASTEXITCODE -ne 0 -or $dirty) { throw 'Keep vendor source edits in the hyper-reV fork before syncing.' }
& git -C $vendor fetch origin $Ref
if ($LASTEXITCODE -ne 0) { throw 'Vendor fetch failed.' }
$commit = & git -C $vendor rev-parse --verify FETCH_HEAD
if ($LASTEXITCODE -ne 0) { throw 'Fetched vendor revision is unavailable.' }
& git -C $vendor checkout --detach $commit
if ($LASTEXITCODE -ne 0) { throw 'Vendor revision checkout failed.' }
if ($Recurse) {
    & git -C $vendor submodule update --init --recursive
    if ($LASTEXITCODE -ne 0) { throw 'Native vendor dependency synchronization failed.' }
}
Write-Output "Pinned hyper-reV at $commit"
& git -C $root diff --submodule=log -- vendor/hyper-reV
if ($LASTEXITCODE -ne 0) { throw 'Vendor diff failed.' }
Write-Output 'Review, test, and commit the vendor pointer. Remote branches stay unchanged.'
