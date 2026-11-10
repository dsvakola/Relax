@echo off
rem Tries the Microsoft Store package on this PC before it is uploaded.
rem Run BUILD-STORE-PACKAGE.bat first. Right-click this file and choose "Run as administrator".
rem It signs a COPY of the package with a test certificate made on this PC (in Release\Store\test; never upload that
rem copy: the real package Release\Store\Relax_*_x64.msix stays unsigned because the Store signs it), installs it,
rem checks the scheduled-backup alias and Task Scheduler, and can run the Windows App Certification Kit (WACK).
rem Please uninstall any other Relax first (Windows Settings > Apps), and use a clean test account if you can.
cd /d "%~dp0"
net session >nul 2>nul || (echo Please right-click TEST-STORE-PACKAGE.bat and choose "Run as administrator". & pause & exit /b 1)
set WACKFLAG=
choice /c YN /m "Also run the Windows App Certification Kit (about 10 minutes)"
if errorlevel 2 (set WACKFLAG=) else (set WACKFLAG=-Wack)
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "store\test-package.ps1" %WACKFLAG%
if errorlevel 1 (echo. & echo SOMETHING FAILED. Please read the PASS/FAIL list above.) else (echo. & echo DONE.)
pause
