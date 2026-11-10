@echo off
title Build Relax - Your data is safe!
echo ======================================================================
echo    BUILDING RELAX - YOUR DATA IS SAFE!  (Tauri 2 + Rust, pnpm)
echo    Developed by: Prof. Dattaraj S. Vidyasagar ^& Prof. Yash D. Vidyasagar
echo    Vidyasagar Academy  -  https://vsa.edu.in/
echo ======================================================================
echo.
cd /d "%~dp0"

echo [1/5] Preparing the static C runtime (Relax then runs on any PC, even without the VC++ runtime)...
if not exist ".cargo" mkdir ".cargo"
> ".cargo\config.toml" (
  echo [target.x86_64-pc-windows-msvc]
  echo rustflags = ["-C", "target-feature=+crt-static"]
)
if errorlevel 1 goto :error

echo [2/5] Installing build tools with pnpm...
call pnpm install
if errorlevel 1 goto :error

echo [3/5] Running the automated engine tests...
pushd src-tauri
cargo test --lib --quiet
if errorlevel 1 (popd & goto :error)
popd

echo [4/5] Building Relax.exe and the installer...
call pnpm tauri build
if errorlevel 1 goto :error

echo [5/5] Copying the results to the "Release" folder...
for /f "tokens=2 delims=:, " %%v in ('findstr /r /c:"\"version\"" src-tauri\tauri.conf.json') do set VERSION=%%~v
if not exist Release mkdir Release
copy /y "src-tauri\target\release\Relax.exe" "Release\Relax-Portable-%VERSION%.exe" >nul
if errorlevel 1 goto :error
for %%f in ("src-tauri\target\release\bundle\nsis\*_%VERSION%_x64-setup.exe") do copy /y /b "%%f" "Release\Relax-Setup-%VERSION%.exe" >nul
if not exist "Release\Relax-Setup-%VERSION%.exe" goto :error

echo.
echo ======================================================================
echo [SUCCESS] Build complete.
echo   Installer: %~dp0Release\Relax-Setup-%VERSION%.exe
echo   Portable : %~dp0Release\Relax-Portable-%VERSION%.exe
echo ======================================================================
pause
exit /b 0

:error
echo.
echo [ERROR] The build stopped. Please read the messages above.
pause
exit /b 1
