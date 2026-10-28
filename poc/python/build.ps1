$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot
& .\.venv\Scripts\python.exe -m PyInstaller --noconfirm --clean --windowed --onedir --name NuPrint --collect-all bleak --collect-all pypdfium2 --collect-all pypdfium2_raw launch_gui.py
if ($LASTEXITCODE -ne 0) { throw 'Build failed' }
Write-Host 'Built dist\NuPrint\NuPrint.exe. Copy the entire NuPrint folder to another Windows PC.'
