@echo off
REM ============================================================================
REM  SaveLink - One-click Windows installer build script
REM
REM  Double-click this file to build the desktop installers.
REM  It will:
REM    1. Locate the MSVC C++ build tools (via vswhere)
REM    2. Make sure Node.js is on PATH
REM    3. Install frontend deps if missing
REM    4. Build + bundle the app, retrying on flaky bundler downloads
REM
REM  Output:
REM    src-tauri\target\release\bundle\nsis\SaveLink_<ver>_x64-setup.exe   (recommended)
REM    src-tauri\target\release\bundle\msi\SaveLink_<ver>_x64_en-US.msi
REM ============================================================================
setlocal
title SaveLink installer build

REM --- always run from the folder this script lives in (the tauri app root) ---
cd /d "%~dp0"

echo.
echo [1/4] Locating MSVC C++ build tools...
set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
if not exist "%VSWHERE%" (
  echo     ERROR: vswhere.exe not found. Visual Studio Build Tools may not be installed.
  echo            Install with: winget install Microsoft.VisualStudio.2022.BuildTools
  goto fail
)
set "VSPATH="
for /f "usebackq tokens=*" %%i in (`"%VSWHERE%" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "VSPATH=%%i"
if not defined VSPATH (
  echo     ERROR: No VS install with the C++ toolset ^(VC.Tools.x86.x64^) was found.
  goto fail
)
set "VCVARS=%VSPATH%\VC\Auxiliary\Build\vcvars64.bat"
if not exist "%VCVARS%" (
  echo     ERROR: vcvars64.bat not found at "%VCVARS%"
  goto fail
)
echo     Using: %VCVARS%
call "%VCVARS%" >nul
if errorlevel 1 ( echo     ERROR: failed to initialize the MSVC environment. & goto fail )

echo.
echo [2/4] Checking Node.js and npm...
where node >nul 2>&1
if errorlevel 1 (
  if exist "%ProgramFiles%\nodejs\node.exe" (
    set "PATH=%ProgramFiles%\nodejs;%PATH%"
  ) else (
    echo     ERROR: Node.js not found. Install with: winget install OpenJS.NodeJS.LTS
    goto fail
  )
)
where npm >nul 2>&1
if errorlevel 1 (
  echo     ERROR: npm not found on PATH ^(it normally ships with Node.js^).
  echo            Reinstall Node.js: winget install OpenJS.NodeJS.LTS
  goto fail
)
for /f "tokens=*" %%v in ('node --version') do echo     Node %%v

echo.
echo [3/4] Ensuring frontend dependencies...
if not exist "node_modules" (
  echo     node_modules missing, running npm install...
  call npm install
  if errorlevel 1 ( echo     ERROR: npm install failed. & goto fail )
) else (
  echo     node_modules present, skipping npm install.
)

echo.
echo [4/4] Building installers (first run is slow: it compiles the whole Rust tree)...
REM Close any running SaveLink instance first -- otherwise the linker cannot
REM overwrite its .exe and fails with LNK1104 "cannot open file".
tasklist /FI "IMAGENAME eq savelink-app.exe" 2>nul | find /I "savelink-app.exe" >nul
if not errorlevel 1 (
  echo     Note: a SaveLink instance is running; closing it so the build can replace its .exe...
  taskkill /IM savelink-app.exe /F >nul 2>&1
  ping -n 3 127.0.0.1 >nul
)
set "MAX_TRIES=5"
set "BUILDLOG=%TEMP%\savelink-build.log"
set /a try=0
:retry
set /a try+=1
echo.
echo ----------------------- build attempt %try% / %MAX_TRIES% -----------------------
REM Capture output so we can classify the failure, while still showing it.
call "node_modules\.bin\tauri.cmd" build > "%BUILDLOG%" 2>&1
set "RC=%ERRORLEVEL%"
type "%BUILDLOG%"
if "%RC%"=="0" goto success
REM Retry ONLY for known-transient causes: dropped WiX/NSIS download, or a locked .exe
REM (LNK1104 / "os error 5" access-denied / "being used by another process" / "failed to
REM remove file" -- usually a running SaveLink or an AV scan). A genuine TypeScript/Rust
REM error is NOT in this list, so we stop on it. Each retry first kills any running app.
findstr /C:"LNK1104" /C:"os error 5" /C:"failed to remove file" /C:"being used by another" /C:"unexpected end of file" /C:"failed to bundle project" "%BUILDLOG%" >nul 2>&1
if errorlevel 1 (
  echo.
  echo     STOPPED: this looks like a real build error, not a transient one -- read it above.
  goto fail
)
if %try% lss %MAX_TRIES% (
  echo     Transient failure ^(dropped download or a locked .exe^). Retrying in ~5s...
  taskkill /IM savelink-app.exe /F >nul 2>&1
  ping -n 6 127.0.0.1 >nul
  goto retry
)
echo.
echo     ERROR: still failing after %MAX_TRIES% attempts. See the log above.
goto fail

:success
echo.
echo ============================================================================
echo  BUILD SUCCEEDED. Installers are here:
echo.
echo    %CD%\src-tauri\target\release\bundle\nsis\   (recommended: *-setup.exe)
echo    %CD%\src-tauri\target\release\bundle\msi\    (*.msi)
echo ============================================================================
start "" "%CD%\src-tauri\target\release\bundle"
echo.
echo Press any key to close...
pause >nul
exit /b 0

:fail
echo.
echo Build did not complete. Press any key to close...
pause >nul
exit /b 1
