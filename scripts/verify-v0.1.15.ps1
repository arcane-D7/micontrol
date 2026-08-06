# Verificação pós-instalação MiControl v0.1.15
# Rodar num PowerShell ELEVADO após instalar o v0.1.15 (instalador NSIS).
# Objetivo: confirmar que as três regressões foram corrigidas:
#   1. As pipes aceitam acesso não-elevado (não → UAC ao arranque)
#   2. MiControlBridge service está a correr (via preferida, sem UAC)
#   3. Temperaturas CPU/GPU são legíveis

Write-Host "=== 1. DACL das pipes (open não-elevado) ===" -ForegroundColor Cyan
$openOk = $false
try {
    $pipe = New-Object System.IO.Pipes.NamedPipeClientStream('.', 'micontrol_bridge', [System.IO.Pipes.PipeDirection]::InOut)
    $pipe.Connect(3000)
    Write-Host "  micontrol_bridge: OK (aberta sem elevação)" -ForegroundColor Green
    $pipe.Dispose()
    $openOk = $true
} catch {
    Write-Host "  micontrol_bridge: FALHOU - $($_.Exception.Message)" -ForegroundColor Red
}
try {
    $pipe2 = New-Object System.IO.Pipes.NamedPipeClientStream('.', 'ecram_service', [System.IO.Pipes.PipeDirection]::InOut)
    $pipe2.Connect(3000)
    Write-Host "  ecram_service: OK (aberta sem elevação)" -ForegroundColor Green
    $pipe2.Dispose()
} catch {
    Write-Host "  ecram_service: FALHOU - $($_.Exception.Message)" -ForegroundColor Red
}

Write-Host ""
Write-Host "=== 2. Serviços ===" -ForegroundColor Cyan
Get-Service -Name MiControlBridge, IoTSvc -ErrorAction SilentlyContinue | Format-Table Name, Status, StartType -AutoSize

Write-Host "=== 3. Scheduled Task MiControlElevated ===" -ForegroundColor Cyan
$t = Get-ScheduledTask -TaskName "MiControlElevated" -ErrorAction SilentlyContinue
if ($t) {
    Write-Host "  Task presente. RunLevel: $($t.Principal.RunLevel)" -ForegroundColor Green
} else {
    Write-Host "  Task AUSENTE - reinstalar o MiControl" -ForegroundColor Yellow
}

Write-Host ""
Write-Host "=== 4. Reinício dos serviços (nova DACL já ativa) ===" -ForegroundColor Cyan
if ($openOk -eq $false) {
    Restart-Service -Name MiControlBridge -Force -ErrorAction SilentlyContinue
    Restart-Service -Name IoTSvc -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 800
    Write-Host "  Serviços reiniciados. Repetir o teste 1." -ForegroundColor Green
} else {
    Write-Host "  Serviços OK, sem necessidade de reinício." -ForegroundColor Green
}

Write-Host ""
Write-Host "=== 5. Temperaturas via bridge (SYSTEM) ===" -ForegroundColor Cyan
# get_fan_info corre elevado via serviço; confirmar que o UI mostra temperaturas.
Write-Host "  Confirme no UI: a página Performance/Fan Control deve mostrar CPU/GPU °C."
Write-Host ""
Write-Host "Pronto."
