# Builds the Microsoft Store package (MSIX) of Relax - Your data is safe!
# Developed by Prof. Dattaraj S. Vidyasagar & Prof. Yash D. Vidyasagar, for Vidyasagar Academy.
#
# Usage (values from Partner Center > your app > Product management > Product identity):
#   .\Build-MSIX.ps1 -IdentityName "12345VidyasagarAcademy.Relax" -Publisher "CN=XXXXXXXX-XXXX-..." -PublisherDisplayName "Vidyasagar Academy"
# Run Build-Relax.bat first, so that src-tauri\target\release\Relax.exe is up to date.
# The Store signs the package itself; the .msix made here is uploaded to Partner Center unsigned.

param(
    [Parameter(Mandatory = $true)][string]$IdentityName,
    [Parameter(Mandatory = $true)][string]$Publisher,
    [Parameter(Mandatory = $true)][string]$PublisherDisplayName
)
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

# Windows SDK tools
$sdk = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\10.*\x64\makeappx.exe" | Sort-Object FullName | Select-Object -Last 1
if (-not $sdk) { throw 'makeappx.exe not found. Please install the Windows 10/11 SDK.' }
$makeappx = $sdk.FullName
$makepri = Join-Path $sdk.DirectoryName 'makepri.exe'

# Version: MSIX needs four parts, and the Store requires the last one to be 0.
$conf = Get-Content 'src-tauri\tauri.conf.json' -Raw | ConvertFrom-Json
$version = "$($conf.version).0"
Write-Host "Packaging Relax $version for the Microsoft Store..."

$exe = 'src-tauri\target\release\Relax.exe'
if (-not (Test-Path $exe)) { throw "$exe not found. Run Build-Relax.bat first." }

# Package layout
$stage = 'store\pkg'
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force "$stage\Assets" | Out-Null
Copy-Item $exe "$stage\Relax.exe"
Copy-Item 'store\Assets\*.png' "$stage\Assets\"
Copy-Item 'NOTICE.txt' "$stage\NOTICE.txt"

$manifest = (Get-Content 'store\AppxManifest.template.xml' -Raw).
    Replace('{{IDENTITY_NAME}}', [Security.SecurityElement]::Escape($IdentityName)).
    Replace('{{PUBLISHER}}', [Security.SecurityElement]::Escape($Publisher)).
    Replace('{{PUBLISHER_DISPLAY_NAME}}', [Security.SecurityElement]::Escape($PublisherDisplayName)).
    Replace('{{VERSION}}', $version)
[IO.File]::WriteAllText((Join-Path $PWD "$stage\AppxManifest.xml"), $manifest, (New-Object Text.UTF8Encoding $false))

# Resource index (lets Windows pick the right logo size for every screen scaling)
& $makepri createconfig /cf "$stage\priconfig.xml" /dq en-US /o | Out-Null
& $makepri new /pr $stage /cf "$stage\priconfig.xml" /mn "$stage\AppxManifest.xml" /of "$stage\resources.pri" /o | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'makepri failed.' }
Remove-Item "$stage\priconfig.xml"

# Package
New-Item -ItemType Directory -Force 'Release' | Out-Null
$out = "Release\Relax-$version-Store.msix"
& $makeappx pack /d $stage /p $out /o
if ($LASTEXITCODE -ne 0) { throw 'makeappx failed.' }
Write-Host ""
Write-Host "[SUCCESS] Store package: $((Resolve-Path $out).Path)"
Write-Host "Upload this file in Partner Center > your app > Packages."

