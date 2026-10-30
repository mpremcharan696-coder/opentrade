# Axiom Solana Memecoin Trading Terminal - Full Stack Launcher
$workspaceRoot = $PSScriptRoot

Write-Host "Starting Axiom Strategy Server in background window..." -ForegroundColor Cyan
Start-Process powershell -ArgumentList "-ExecutionPolicy Bypass -NoExit -File `"$workspaceRoot\start-server.ps1`""

Start-Sleep -Seconds 2

Write-Host "Starting Axiom Native Desktop App..." -ForegroundColor Green
powershell -ExecutionPolicy Bypass -File "$workspaceRoot\start-desktop.ps1"
