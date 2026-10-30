# Axiom Solana Memecoin Trading Terminal - Strategy Server Launcher
$ErrorActionPreference = "Stop"

$workspaceRoot = $PSScriptRoot
Set-Location $workspaceRoot

# Ensure Rust and OpenSSL tools are in PATH
$env:PATH = "C:\Program Files\nodejs;C:\Users\mprem\.cargo\bin;C:\Users\mprem\openssl\bin;" + $env:PATH
$env:OPENSSL_DIR = "C:\Users\mprem\openssl"
$env:OPENSSL_INCLUDE_DIR = "C:\Users\mprem\openssl\include"
$env:OPENSSL_LIB_DIR = "C:\Users\mprem\openssl\lib"

Write-Host "============================================================" -ForegroundColor Cyan
Write-Host "  Launching Axiom Strategy Server (http://localhost:8080)" -ForegroundColor Green
Write-Host "============================================================" -ForegroundColor Cyan

cargo run -p trading-server
