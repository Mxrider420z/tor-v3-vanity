@echo off
REM Tor V3 Vanity Address Generator - Windows Build
REM Just double-click this file to build both CLI and GUI

cd /d "%~dp0"

echo.
echo === Tor V3 Vanity Generator - Windows Build ===
echo.
echo Running build script...
echo.

powershell -ExecutionPolicy Bypass -File "%~dp0build-windows.ps1"

if %ERRORLEVEL% neq 0 (
    echo.
    echo Build failed! Press any key to exit...
    pause > nul
    exit /b 1
)

echo.
echo Build complete! Press any key to exit...
pause > nul
