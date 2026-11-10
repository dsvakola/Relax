@echo off
rem Builds the Microsoft Store package of Relax - Your data is safe!
rem   Result: Release\Store\Relax_<version>_x64.msix   (UNSIGNED: this is the file to upload in Partner Center;
rem   the Store signs it itself).
rem Needs once on this PC: Rust, Visual Studio Build Tools (C++ with the Windows SDK), Node.js and pnpm
rem (see README.md), and the Identity Name pasted into store\identity.txt from Partner Center.
rem The version number comes from src-tauri\tauri.conf.json: change it for every update.
setlocal EnableDelayedExpansion
title Build Relax - Microsoft Store package
cd /d "%~dp0"
where cargo >nul 2>nul || (echo Rust is not installed on this PC yet. See README.md. & goto failed)
where pnpm >nul 2>nul || (echo pnpm is not installed on this PC yet. See README.md. & goto failed)

rem The C runtime goes inside the EXE, so Relax also runs on a fresh Windows PC.
if not exist ".cargo" mkdir ".cargo"
> ".cargo\config.toml" (
  echo [target.x86_64-pc-windows-msvc]
  echo rustflags = ["-C", "target-feature=+crt-static"]
)

echo [1/4] Installing build tools with pnpm...
call pnpm install
if errorlevel 1 goto failed

echo [2/4] Running the automated engine tests...
pushd src-tauri
cargo test --lib --quiet
if errorlevel 1 (popd & goto failed)
popd

echo [3/4] Building Relax.exe (no installer needed for the Store)...
call pnpm tauri build --no-bundle
if errorlevel 1 goto failed

echo [4/4] Making the Store package...
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "store\make-package.ps1"
if errorlevel 1 goto failed

echo.
echo =====================================================
echo DONE. Upload the file in Release\Store\ named Relax_*_x64.msix in Partner Center.
echo To try it on this PC first, run TEST-STORE-PACKAGE.bat.
echo =====================================================
if /i not "%~1"=="nopause" pause
exit /b 0

:failed
echo.
echo BUILD FAILED. Please read the messages above.
if /i not "%~1"=="nopause" pause
exit /b 1
