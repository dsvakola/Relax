# Used by BUILD-STORE-PACKAGE.bat. Packs the already built Relax.exe into the Microsoft Store package (.msix).
# Developed by Prof. Dattaraj S. Vidyasagar & Prof. Yash D. Vidyasagar, for Vidyasagar Academy.
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path -Parent $PSScriptRoot)

function Read-Identity([string]$key) {
    $line = Get-Content 'store\identity.txt' | Where-Object { $_ -match "^\s*$key\s*=" } | Select-Object -First 1
    if (-not $line) { throw "$key is missing in store\identity.txt." }
    return ($line -replace "^\s*$key\s*=\s*", '').Trim()
}
$name = Read-Identity 'NAME'; $publisher = Read-Identity 'PUBLISHER'; $pubDisplay = Read-Identity 'PUBLISHER_DISPLAY'
if ($name -like '*PASTE-*' -or $name -eq '') { throw 'Please paste the Identity Name from Partner Center into NAME= in store\identity.txt first.' }

# Windows SDK packaging tools
$kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
$sdk = Get-ChildItem $kits -Directory -Filter '10.*' -ErrorAction SilentlyContinue | Sort-Object Name -Descending |
    ForEach-Object { Join-Path $_.FullName 'x64' } | Where-Object { Test-Path (Join-Path $_ 'makeappx.exe') } | Select-Object -First 1
if (-not $sdk) { throw 'makeappx.exe was not found. In Visual Studio Installer choose Modify and tick "Windows 11 SDK".' }

# Version: tauri.conf.json is the single source. The Store needs four parts and the last one must be 0.
$conf = Get-Content 'src-tauri\tauri.conf.json' -Raw | ConvertFrom-Json
$version = "$($conf.version).0"
$exe = 'src-tauri\target\release\Relax.exe'
if (-not (Test-Path $exe)) { throw "$exe was not found." }
Write-Host "Packaging Relax $version for the Microsoft Store..."

$stage = 'target\msix'
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force "$stage\Assets" | Out-Null
Copy-Item $exe "$stage\Relax.exe"
Copy-Item 'store\Assets\*.png' "$stage\Assets\"
Copy-Item 'NOTICE.txt' "$stage\NOTICE.txt"

$esc = { param($s) [Security.SecurityElement]::Escape($s) }
$manifest = (Get-Content 'store\AppxManifest.template.xml' -Raw -Encoding UTF8).
    Replace('{{IDENTITY_NAME}}', (& $esc $name)).
    Replace('{{PUBLISHER}}', (& $esc $publisher)).
    Replace('{{PUBLISHER_DISPLAY_NAME}}', (& $esc $pubDisplay)).
    Replace('{{VERSION}}', $version)
[IO.File]::WriteAllText((Join-Path $PWD "$stage\AppxManifest.xml"), $manifest, (New-Object Text.UTF8Encoding $false))

# Resource index: lets Windows pick the sharpest logo for every screen scaling (100% to 400%)
& (Join-Path $sdk 'makepri.exe') createconfig /cf "$stage\priconfig.xml" /dq en-US /pv 10.0.0 /o | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'makepri createconfig failed.' }
& (Join-Path $sdk 'makepri.exe') new /pr $stage /cf "$stage\priconfig.xml" /mn "$stage\AppxManifest.xml" /of "$stage\resources.pri" /o | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'makepri failed.' }
Remove-Item "$stage\priconfig.xml"

New-Item -ItemType Directory -Force 'Release\Store' | Out-Null
$out = "Release\Store\Relax_${version}_x64.msix"
& (Join-Path $sdk 'makeappx.exe') pack /d $stage /p $out /o | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'makeappx failed.' }
Write-Host "Store package: $((Resolve-Path $out).Path)"
