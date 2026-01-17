@echo off
REM Tor V3 Vanity Generator - Complete Build
REM Builds GUI + CUDA generator

cd /d "%~dp0"
echo.
echo === Building Tor V3 Vanity Generator ===
echo.
echo This will build both:
echo   1. t3v-gui.exe (GUI)
echo   2. vanity_torv3_cuda.exe (GPU accelerator)
echo.
powershell -ExecutionPolicy Bypass -File "%~dp0build-all-windows.ps1" %*
if %ERRORLEVEL% neq 0 (
    echo.
    echo Build encountered errors. Press any key to exit...
    pause > nul
    exit /b 1
)
echo.
echo Press any key to exit...
pause > nul
