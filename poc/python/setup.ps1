$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot
if (Get-Command uv -ErrorAction SilentlyContinue) {
    uv venv .venv --python 3.14
    if ($LASTEXITCODE -ne 0) { throw 'Could not create the Python environment' }
    uv pip install --python .venv\Scripts\python.exe -e '.[dev]'
} else {
    python -m venv .venv
    if ($LASTEXITCODE -ne 0) { throw 'Install Python 3.11 or later with tkinter, then rerun setup.ps1' }
    & .\.venv\Scripts\python.exe -m pip install -e '.[dev]'
}
if ($LASTEXITCODE -ne 0) { throw 'Dependency installation failed' }
Write-Host 'Ready. Double-click Start NuPrint.cmd.'
