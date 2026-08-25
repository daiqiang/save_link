@echo off
REM ============================================================================
REM  SaveLink - One-click Windows portable build script
REM
REM  Double-click this file to build the no-install portable package.
REM  Output:
REM    src-tauri\target\release\bundle\portable\
REM      SaveLink_<ver>_windows_x64_portable.zip
REM      SaveLink_<ver>_windows_x64_portable.zip.sha256.txt
REM ============================================================================
setlocal
title SaveLink portable build

REM Always run from the folder this script lives in (the Tauri app root).
cd /d "%~dp0"

echo.
echo [1/5] Locating MSVC C++ build tools...
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
echo [2/5] Checking Node.js, npm, and Rust...
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
  goto fail
)
where cargo >nul 2>&1
if errorlevel 1 (
  echo     ERROR: Rust/Cargo not found. Install from: https://rustup.rs/
  goto fail
)
for /f "tokens=*" %%v in ('node --version') do echo     Node %%v
for /f "tokens=*" %%v in ('cargo --version') do echo     %%v

echo.
echo [3/5] Ensuring frontend dependencies...
if not exist "node_modules" (
  echo     node_modules missing, running npm install...
  call npm install
  if errorlevel 1 ( echo     ERROR: npm install failed. & goto fail )
) else (
  echo     node_modules present, skipping npm install.
)

for /f "usebackq tokens=*" %%v in (`node -p "require('./src-tauri/tauri.conf.json').version"`) do set "APP_VERSION=%%v"
if not defined APP_VERSION (
  echo     ERROR: could not read version from src-tauri\tauri.conf.json.
  goto fail
)

echo.
echo [4/5] Building SaveLink %APP_VERSION% without MSI/NSIS bundling...
echo     Closing any running SaveLink build from this workspace...
powershell -NoProfile -ExecutionPolicy Bypass -Command "try { $root=(Get-Location).Path; $paths=@((Join-Path $root 'src-tauri\target\release\savelink-app.exe'),(Join-Path $root 'src-tauri\target\release\bundle\portable\SaveLink\SaveLink.exe')); $targets=@(Get-Process -Name 'savelink-app','SaveLink' -ErrorAction SilentlyContinue | Where-Object { $_.Path -in $paths }); if ($targets.Count -gt 0) { $targets | Stop-Process -Force -ErrorAction Stop; Start-Sleep -Milliseconds 500 }; exit 0 } catch { Write-Error $_; exit 1 }"
if errorlevel 1 (
  echo     ERROR: failed to close a running SaveLink process.
  goto fail
)
ping -n 3 127.0.0.1 >nul
call "node_modules\.bin\tauri.cmd" build --no-bundle --ci
if errorlevel 1 (
  echo     ERROR: portable release build failed.
  goto fail
)

echo.
echo [5/5] Creating portable folder and zip...
set "RELEASE_EXE=%CD%\src-tauri\target\release\savelink-app.exe"
set "README_SOURCE=%CD%\packaging\portable\README.txt"
set "RESOURCE_SOURCE=%CD%\src-tauri\resources"
set "PORTABLE_ROOT=%CD%\src-tauri\target\release\bundle\portable"
set "PORTABLE_DIR=%PORTABLE_ROOT%\SaveLink"
set "ZIP_PATH=%PORTABLE_ROOT%\SaveLink_%APP_VERSION%_windows_x64_portable.zip"
set "HASH_PATH=%ZIP_PATH%.sha256.txt"

if not exist "%RELEASE_EXE%" (
  echo     ERROR: release executable not found at "%RELEASE_EXE%".
  goto fail
)
if not exist "%README_SOURCE%" (
  echo     ERROR: portable README not found at "%README_SOURCE%".
  goto fail
)
if not exist "%RESOURCE_SOURCE%\manifest.db" (
  echo     ERROR: Steam manifest database not found at "%RESOURCE_SOURCE%\manifest.db".
  goto fail
)
if not exist "%PORTABLE_DIR%" mkdir "%PORTABLE_DIR%"
copy /Y "%RELEASE_EXE%" "%PORTABLE_DIR%\SaveLink.exe" >nul
if errorlevel 1 (
  echo     ERROR: failed to stage SaveLink.exe.
  goto fail
)
copy /Y "%README_SOURCE%" "%PORTABLE_DIR%\README.txt" >nul
if errorlevel 1 (
  echo     ERROR: failed to stage README.txt.
  goto fail
)
xcopy /E /I /Y "%RESOURCE_SOURCE%" "%PORTABLE_DIR%\resources" >nul
if errorlevel 1 (
  echo     ERROR: failed to stage Steam discovery resources.
  goto fail
)

powershell -NoProfile -ExecutionPolicy Bypass -Command "$ErrorActionPreference='Stop'; Compress-Archive -LiteralPath $env:PORTABLE_DIR -DestinationPath $env:ZIP_PATH -Force; $stream=[IO.File]::OpenRead($env:ZIP_PATH); try { $sha=[Security.Cryptography.SHA256]::Create(); try { $hash=[BitConverter]::ToString($sha.ComputeHash($stream)).Replace('-','') } finally { $sha.Dispose() } } finally { $stream.Dispose() }; Set-Content -LiteralPath $env:HASH_PATH -Value ($hash + '  ' + [IO.Path]::GetFileName($env:ZIP_PATH)) -Encoding ascii; Write-Host ('    SHA-256: ' + $hash)"
if errorlevel 1 (
  echo     ERROR: failed to create the portable zip or checksum.
  goto fail
)

echo.
echo ============================================================================
echo  PORTABLE BUILD SUCCEEDED
echo.
echo    Folder: %PORTABLE_DIR%
echo    Zip:    %ZIP_PATH%
echo    SHA256: %HASH_PATH%
echo ============================================================================
if /I "%~1"=="--no-open" exit /b 0
start "" "%PORTABLE_ROOT%"
echo.
echo Press any key to close...
pause >nul
exit /b 0

:fail
echo.
echo Portable build did not complete.
if /I "%~1"=="--no-open" exit /b 1
echo Press any key to close...
pause >nul
exit /b 1
