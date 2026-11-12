#Requires -Version 7.0
<#
.SYNOPSIS
  Build the portable Windows zip of pandite.

.DESCRIPTION
  Stages pandite.exe, the license, the third-party notices and the README in
  a folder named pandite-<version>-<target> and compresses it to
  <OutDir>\pandite-<version>-<target>.zip.

.EXAMPLE
  pwsh packaging/windows/build-zip.ps1 -Exe target/x86_64-pc-windows-msvc/release/pandite.exe `
    -Version 0.1.0 -Target x86_64-pc-windows-msvc -OutDir dist
#>
param(
  [Parameter(Mandatory = $true)][string]$Exe,
  [Parameter(Mandatory = $true)][string]$Version,
  [Parameter(Mandatory = $true)][string]$Target,
  [string]$OutDir = 'dist'
)

$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..\..')

if (-not (Test-Path -LiteralPath $Exe -PathType Leaf)) { throw "binary not found: $Exe" }

$name = "pandite-$Version-$Target"
$work = Join-Path ([System.IO.Path]::GetTempPath()) ("pandite-zip-" + [System.Guid]::NewGuid().ToString('N'))
$stage = Join-Path $work $name
New-Item -ItemType Directory -Force -Path $stage | Out-Null
try {
  Copy-Item -LiteralPath $Exe -Destination (Join-Path $stage 'pandite.exe')
  Copy-Item -LiteralPath (Join-Path $root 'LICENSE') -Destination (Join-Path $stage 'LICENSE.txt')
  Copy-Item -LiteralPath (Join-Path $root 'README.md') -Destination (Join-Path $stage 'README.md')
  Copy-Item -LiteralPath (Join-Path $root 'docs\THIRD-PARTY-NOTICES.md') -Destination (Join-Path $stage 'THIRD-PARTY-NOTICES.md')

  New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
  $zip = Join-Path (Resolve-Path $OutDir) "$name.zip"
  if (Test-Path -LiteralPath $zip) { Remove-Item -LiteralPath $zip -Force }
  Add-Type -AssemblyName System.IO.Compression.FileSystem
  [System.IO.Compression.ZipFile]::CreateFromDirectory($stage, $zip, [System.IO.Compression.CompressionLevel]::Optimal, $true)
  Write-Output "wrote $zip"
}
finally {
  Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}
