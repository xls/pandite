#Requires -Version 7.0
<#
.SYNOPSIS
  Build the Windows setup program of Pandite with Inno Setup 6.

.DESCRIPTION
  Compiles packaging/windows/pandite.iss into
  <OutDir>\pandite-<version>-<target>-setup.exe. The setup program installs
  the files of the portable zip for the current user.

  ISCC.exe comes from -Iscc, else from $env:PANDITE_ISCC, else from the
  standard Inno Setup 6 folders, the Inno Setup 6 uninstall entry or PATH.
  The script stops when it finds none. Inno Setup 6.3 or later is required.

  -FindIscc prints the ISCC.exe path the script would use, or nothing, and
  builds nothing.

.EXAMPLE
  pwsh packaging/windows/build-setup.ps1 -Exe target/release/pandite.exe `
    -Version 0.1.0 -Target x86_64-pc-windows-msvc -OutDir dist

.EXAMPLE
  pwsh packaging/windows/build-setup.ps1 -FindIscc
#>
[CmdletBinding(DefaultParameterSetName = 'Build')]
param(
  [Parameter(ParameterSetName = 'Build', Mandatory = $true)][string]$Exe,
  [Parameter(ParameterSetName = 'Build', Mandatory = $true)][string]$Version,
  [Parameter(ParameterSetName = 'Build', Mandatory = $true)][string]$Target,
  [Parameter(ParameterSetName = 'Build')][string]$OutDir = 'dist',
  [Parameter(ParameterSetName = 'Build')][string]$Iscc = '',
  [Parameter(ParameterSetName = 'Find', Mandatory = $true)][switch]$FindIscc
)

$ErrorActionPreference = 'Stop'

function Find-Iscc {
  $candidates = [System.Collections.Generic.List[string]]::new()
  if ($env:PANDITE_ISCC) { $candidates.Add($env:PANDITE_ISCC) }
  foreach ($base in @(${env:ProgramFiles(x86)}, $env:ProgramFiles, $(if ($env:LOCALAPPDATA) { Join-Path $env:LOCALAPPDATA 'Programs' }))) {
    if ($base) { $candidates.Add((Join-Path $base 'Inno Setup 6\ISCC.exe')) }
  }
  foreach ($key in @(
      'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Inno Setup 6_is1',
      'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Inno Setup 6_is1',
      'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Inno Setup 6_is1')) {
    $location = (Get-ItemProperty -LiteralPath $key -Name InstallLocation -ErrorAction SilentlyContinue).InstallLocation
    if ($location) { $candidates.Add((Join-Path $location 'ISCC.exe')) }
  }
  $onPath = Get-Command 'ISCC.exe' -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
  if ($onPath) { $candidates.Add($onPath.Source) }
  foreach ($candidate in $candidates) {
    if (Test-Path -LiteralPath $candidate -PathType Leaf) { return (Resolve-Path -LiteralPath $candidate).Path }
  }
  return $null
}

if ($FindIscc) {
  $found = Find-Iscc
  if ($found) { Write-Output $found }
  exit 0
}

if ($Version -notmatch '^\d+\.\d+\.\d+([-+][0-9A-Za-z.+-]+)?$') { throw "version is not x.y.z[-pre][+build]: $Version" }
$arch = switch ($Target) {
  'x86_64-pc-windows-msvc' { 'x64' }
  'aarch64-pc-windows-msvc' { 'arm64' }
  default { throw "no setup program for target $Target (x86_64-pc-windows-msvc or aarch64-pc-windows-msvc)" }
}
if (-not (Test-Path -LiteralPath $Exe -PathType Leaf)) { throw "binary not found: $Exe" }

if (-not $Iscc) { $Iscc = Find-Iscc }
if (-not $Iscc -or -not (Test-Path -LiteralPath $Iscc -PathType Leaf)) {
  throw 'ISCC.exe not found. Install Inno Setup 6.3 or later from https://jrsoftware.org/isdl.php, or pass -Iscc <path to ISCC.exe>.'
}

# VersionInfoVersion takes numbers only: drop a pre-release or build suffix.
$numeric = (($Version -split '[-+]')[0]) + '.0'
$exePath = (Resolve-Path -LiteralPath $Exe).Path
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$out = (Resolve-Path -LiteralPath $OutDir).Path
$script = Join-Path $PSScriptRoot 'pandite.iss'

Write-Output "ISCC.exe: $Iscc"
& $Iscc /Qp "/DVersion=$Version" "/DNumericVersion=$numeric" "/DTarget=$Target" "/DArch=$arch" "/DExe=$exePath" "/DOutDir=$out" $script
if ($LASTEXITCODE -ne 0) { throw "ISCC.exe failed with exit code $LASTEXITCODE" }

$setup = Join-Path $out "pandite-$Version-$Target-setup.exe"
if (-not (Test-Path -LiteralPath $setup -PathType Leaf)) { throw "ISCC.exe did not write $setup" }
Write-Output "wrote $setup"
