# Used by TEST-STORE-PACKAGE.bat. TEST ONLY: the signed copy it makes is never uploaded to the Store.
# Developed by Prof. Dattaraj S. Vidyasagar & Prof. Yash D. Vidyasagar, for Vidyasagar Academy.
param([switch]$Wack)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path -Parent $PSScriptRoot)

function Read-Identity([string]$key) {
    $line = Get-Content 'store\identity.txt' | Where-Object { $_ -match "^\s*$key\s*=" } | Select-Object -First 1
    return ($line -replace "^\s*$key\s*=\s*", '').Trim()
}
$pub = Read-Identity 'PUBLISHER'; $name = Read-Identity 'NAME'
$conf = Get-Content 'src-tauri\tauri.conf.json' -Raw | ConvertFrom-Json
$ver = "$($conf.version).0"
$src = "Release\Store\Relax_${ver}_x64.msix"
if (-not (Test-Path $src)) { throw "$src was not found. Run BUILD-STORE-PACKAGE.bat first." }

$kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10'
$signtool = Get-ChildItem (Join-Path $kits 'bin') -Directory -Filter '10.*' | Sort-Object Name -Descending |
    ForEach-Object { Join-Path $_.FullName 'x64\signtool.exe' } | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $signtool) { throw 'signtool.exe (Windows SDK) was not found.' }

# Test certificate with the same publisher name as the Store identity (made once on this PC)
$cert = Get-ChildItem Cert:\CurrentUser\My | Where-Object { $_.Subject -eq $pub -and $_.FriendlyName -eq 'Relax Store test' } | Select-Object -First 1
if (-not $cert) {
    $cert = New-SelfSignedCertificate -Type Custom -Subject $pub -KeyUsage DigitalSignature -FriendlyName 'Relax Store test' `
        -CertStoreLocation Cert:\CurrentUser\My -NotAfter (Get-Date).AddYears(2) `
        -TextExtension @('2.5.29.37={text}1.3.6.1.5.5.7.3.3', '2.5.29.19={text}')
}
$out = 'Release\Store\test'
New-Item -ItemType Directory -Force $out | Out-Null
Export-Certificate -Cert $cert -FilePath "$out\relax-test.cer" | Out-Null
Import-Certificate -FilePath "$out\relax-test.cer" -CertStoreLocation Cert:\LocalMachine\TrustedPeople | Out-Null

$test = "$out\Relax_${ver}_x64.msix"
Copy-Item $src $test -Force
& $signtool sign /q /fd SHA256 /sha1 $cert.Thumbprint $test
if ($LASTEXITCODE -ne 0) { throw 'Signing the test copy failed.' }

# Remove an older Relax test/Store install, then install this one
Get-AppxPackage -Name $name -ErrorAction SilentlyContinue | Remove-AppxPackage -ErrorAction SilentlyContinue
Add-AppxPackage -Path $test -ForceApplicationShutdown -ForceUpdateFromAnyVersion
$pkg = Get-AppxPackage -Name $name | Select-Object -First 1
if (-not $pkg) { throw 'The package did not install.' }
Write-Host "Installed: $($pkg.PackageFullName)"

# ---- Check 1: no administrator rights are asked for ----
$results = [ordered]@{}
$manifest = Get-Content (Join-Path $pkg.InstallLocation 'AppxManifest.xml') -Raw
$results['Manifest asks only for runFullTrust (no allowElevation)'] = ($manifest -match 'runFullTrust') -and ($manifest -notmatch 'allowElevation')

# ---- Check 2: the command-line alias used by Task Scheduler runs a backup ----
$alias = Join-Path $env:LOCALAPPDATA 'Microsoft\WindowsApps\RelaxBackup.exe'
$results['Alias RelaxBackup.exe exists'] = Test-Path $alias
$work = Join-Path $env:TEMP 'RelaxStoreTest'
if (Test-Path $work) { Remove-Item $work -Recurse -Force }
New-Item -ItemType Directory "$work\Source", "$work\Target" -Force | Out-Null
Set-Content "$work\Source\hello.txt" 'Relax Store test file'
$job = "$work\Store test.relax"
$jobJson = @{ settings = @{ pairs = @(@{ left = "$work\Source"; right = "$work\Target" }); syncVariant = 'update' } } | ConvertTo-Json -Depth 5
[IO.File]::WriteAllText($job, $jobJson, (New-Object Text.UTF8Encoding $false))
$pkgData = Join-Path $env:LOCALAPPDATA "Packages\$($pkg.PackageFamilyName)\LocalCache\Roaming\in.edu.vsa.relax"
$p = Start-Process $alias -ArgumentList '--run', "`"$job`"" -Wait -PassThru
$results['Backup run through the alias finished with exit code 0'] = ($p.ExitCode -eq 0)
$results['  ...and copied the file'] = Test-Path "$work\Target\hello.txt"
$results['  ...and wrote a log in the package data folder'] = (Test-Path "$pkgData\logs") -and ((Get-ChildItem "$pkgData\logs" -ErrorAction SilentlyContinue | Measure-Object).Count -gt 0)

# ---- Check 3: a Task Scheduler task starts Relax through the alias ----
Remove-Item "$work\Target\hello.txt" -Force -ErrorAction SilentlyContinue
$tn = 'Relax\StoreTest'
schtasks.exe /Create /F /TN $tn /SC ONCE /ST 23:59 /TR "`"$alias`" --run `"$job`"" | Out-Null
$results['Task Scheduler task created'] = ($LASTEXITCODE -eq 0)
schtasks.exe /Run /TN $tn | Out-Null
$ok = $false
for ($i = 0; $i -lt 30 -and -not $ok; $i++) { Start-Sleep 1; $ok = Test-Path "$work\Target\hello.txt" }
$results['Task Scheduler ran the backup from the installed Store package'] = $ok
schtasks.exe /Delete /F /TN $tn | Out-Null

Write-Host ''
Write-Host '================ RESULTS ================'
$fail = 0
foreach ($k in $results.Keys) { $s = if ($results[$k]) { 'PASS' } else { $fail++; 'FAIL' }; Write-Host ("{0}  {1}" -f $s, $k) }
Write-Host "Data folder inside the package (settings, logs, sync database): $pkgData"
Write-Host 'Now start Relax from the Start menu and check the screens, Settings, Save job, Schedule and Logs.'
Start-Process explorer.exe "shell:AppsFolder\$($pkg.PackageFamilyName)!Relax"

# ---- Windows App Certification Kit (takes about 10 minutes) ----
if ($Wack) {
    $appcert = Join-Path $kits 'App Certification Kit\appcert.exe'
    if (-not (Test-Path $appcert)) { throw 'appcert.exe was not found (Windows App Certification Kit).' }
    Write-Host 'Running the Windows App Certification Kit. Please wait, do not touch the PC...'
    $report = Join-Path $PWD "$out\wack-report.xml"
    & $appcert reset | Out-Null
    & $appcert test -appxpackagepath (Join-Path $PWD $test) -reportoutputpath $report
    if (Test-Path $report) {
        $x = [xml](Get-Content $report -Raw)
        Write-Host "WACK overall result: $($x.REPORT.OVERALL_RESULT)   (report: $report)"
    }
}
if ($fail -gt 0) { exit 1 }
