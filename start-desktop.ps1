# Axiom Solana Memecoin Trading Terminal - Desktop Launcher
$ErrorActionPreference = "Stop"

$workspaceRoot = $PSScriptRoot
$desktopDir = Join-Path $workspaceRoot "apps\desktop"

# Ensure Node, Cargo, and OpenSSL are in PATH for this session
$env:PATH = "C:\Program Files\nodejs;C:\Users\mprem\.cargo\bin;C:\Users\mprem\openssl\bin;" + $env:PATH
$env:OPENSSL_DIR = "C:\Users\mprem\openssl"
$env:OPENSSL_INCLUDE_DIR = "C:\Users\mprem\openssl\include"
$env:OPENSSL_LIB_DIR = "C:\Users\mprem\openssl\lib"

Write-Host "============================================================" -ForegroundColor Cyan
Write-Host "  Launching Axiom Solana Terminal (Native Desktop Window)" -ForegroundColor Green
Write-Host "============================================================" -ForegroundColor Cyan

# Clean up any lingering process on port 1420
Get-NetTCPConnection -LocalPort 1420 -ErrorAction SilentlyContinue | ForEach-Object {
    Stop-Process -Id $_.OwningProcess -Force -ErrorAction SilentlyContinue
}
Start-Sleep -Milliseconds 300

Set-Location $desktopDir
npm.cmd run tauri dev
