# Package dist\windows\Moo (from scripts/package-windows.sh) as an MSIX for the Microsoft Store, on
# Windows:
#   pwsh scripts/package-msix.ps1 -Version 0.9.0 -Name 12345Moo.Moo -Publisher "CN=..." -PublisherName "..."
#   -> dist\release\Moo-windows-x64.msix (unsigned: upload it to Partner Center, which signs it)
# Name, Publisher and PublisherName are on Partner Center's Product identity page. To try it
# locally, turn on Developer Mode and run Add-AppxPackage -AllowUnsigned.
param(
  [Parameter(Mandatory = $true)][string]$Version,
  [string]$Name = "Moo.Moo",
  [string]$Publisher = "CN=Moo",
  [string]$PublisherName = "Moo"
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$src = Join-Path $root "dist\windows\Moo"
$layout = Join-Path $root "dist\windows\msix"
$out = Join-Path $root "dist\release\Moo-windows-x64.msix"
if (-not (Test-Path (Join-Path $src "moo.exe"))) { throw "no $src\moo.exe: run scripts/package-windows.sh first" }

Remove-Item -Recurse -Force $layout -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force (Join-Path $layout "Assets") | Out-Null
Copy-Item -Recurse (Join-Path $src "*") $layout
# Execution aliases must point at an .exe: the console copy goes in as moo-cli.exe.
Move-Item (Join-Path $layout "moo.com") (Join-Path $layout "moo-cli.exe")
Copy-Item (Join-Path $root "packaging\windows\*.png") (Join-Path $layout "Assets")
$v = (($Version -split '[-+]')[0] -split '\.') + @('0', '0', '0') | Select-Object -First 3
$manifest = (Get-Content -Raw (Join-Path $root "packaging\windows\AppxManifest.xml")).Replace('$VERSION$', ($v -join '.') + '.0').Replace('$NAME$', $Name).Replace('$PUBLISHER_NAME$', $PublisherName).Replace('$PUBLISHER$', $Publisher)
Set-Content -Encoding utf8 (Join-Path $layout "AppxManifest.xml") $manifest

# The newest Windows SDK's tools.
$sdk = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\10.*\x64" | Sort-Object Name -Descending | Select-Object -First 1
$makeappx = Join-Path $sdk.FullName "makeappx.exe"
New-Item -ItemType Directory -Force (Split-Path $out) | Out-Null
& $makeappx pack /o /d $layout /p $out
if ($LASTEXITCODE -ne 0) { throw "makeappx failed" }

Get-Item $out | Select-Object FullName, Length
