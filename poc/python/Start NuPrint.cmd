@echo off
cd /d "%~dp0"
if exist "dist\NuPrint\NuPrint.exe" (
    start "" "dist\NuPrint\NuPrint.exe"
    exit /b
)
if exist ".venv\Scripts\pythonw.exe" (
    start "" ".venv\Scripts\pythonw.exe" "launch_gui.py"
    exit /b
)
echo Run setup.ps1 first to install NuPrint.
pause
