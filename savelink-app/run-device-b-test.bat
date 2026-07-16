@echo off
setlocal
cd /d "%~dp0"

set "SAVELINK_TEST_DATA_DIR=%APPDATA%\com.daiq.savelink-device-b-test"
set "SAVELINK_EXE=%CD%\src-tauri\target\release\savelink-app.exe"

if not exist "%SAVELINK_EXE%" (
  echo SaveLink release executable was not found.
  echo Run build-installer.bat first, then run this script again.
  pause
  exit /b 1
)

echo Starting isolated SaveLink device B profile:
echo   %SAVELINK_TEST_DATA_DIR%
start "" "%SAVELINK_EXE%"
exit /b 0
