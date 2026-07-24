; MiControl — custom NSIS installer hooks
; Adds:
;   • Privacy & Open-Source notice page
;   • Options page (desktop shortcut + startup — both pre-selected)
;   • Hardware driver installation (VirtualControlHID + IoTDriver)

; ── Global variables ──────────────────────────────────────────────────────────
Var DesktopCB      ; HWND of desktop-shortcut checkbox
Var StartupCB      ; HWND of startup checkbox
Var DoDesktop      ; ${BST_CHECKED} / ${BST_UNCHECKED}
Var DoStartup      ; ${BST_CHECKED} / ${BST_UNCHECKED}

; ── Page 1: Privacy & Open-Source notice ─────────────────────────────────────
Function InfoPage
  nsDialogs::Create 1018
  Pop $0

  ${NSD_CreateLabel} 0 0 100% 12u "Informações importantes antes de continuar:"
  Pop $0

  ${NSD_CreateGroupBox} 0 16u 100% 108u "Privacidade & Licença"
  Pop $0

  ${NSD_CreateLabel} 10u 30u 92% 88u \
    "• Todos os seus dados são mantidos ESTRITAMENTE NO SEU COMPUTADOR.$\r$\n  Nenhuma informação é transmitida para servidores externos.$\r$\n$\r$\n• MiControl é um software TOTALMENTE GRATUITO e SEM FINS LUCRATIVOS.$\r$\n  É open source — a sua distribuição é e deve ser sempre gratuita.$\r$\n$\r$\n• Código-fonte disponível em: github.com/Freitas-MA$\r$\n• Desenvolvido por: Marcos Freitas"
  Pop $0

  nsDialogs::Show
FunctionEnd

Function InfoPageLeave
FunctionEnd

; ── Page 2: Installation options ─────────────────────────────────────────────
Function OptionsPage
  nsDialogs::Create 1018
  Pop $0

  ${NSD_CreateLabel} 0 0 100% 12u "Opções de instalação:"
  Pop $0

  ${NSD_CreateCheckBox} 0 20u 100% 14u "Criar atalho no Ambiente de Trabalho"
  Pop $DesktopCB
  ${NSD_SetState} $DesktopCB ${BST_CHECKED}

  ${NSD_CreateCheckBox} 0 42u 100% 14u "Iniciar o MiControl automaticamente com o Windows"
  Pop $StartupCB
  ${NSD_SetState} $StartupCB ${BST_CHECKED}

  nsDialogs::Show
FunctionEnd

Function OptionsPageLeave
  ${NSD_GetState} $DesktopCB $DoDesktop
  ${NSD_GetState} $StartupCB $DoStartup
FunctionEnd

; ── Macros ────────────────────────────────────────────────────────────────────

!macro customHeader
  !include "nsDialogs.nsh"
!macroend

!macro customPageBefore
  Page custom InfoPage InfoPageLeave
  Page custom OptionsPage OptionsPageLeave
!macroend

!macro customInstall
  ; ── Desktop shortcut (user choice) ────────────────────────────────────────
  ${If} $DoDesktop == ${BST_CHECKED}
    CreateShortcut "$DESKTOP\MiControl.lnk" "$INSTDIR\micontrol.exe" "" "$INSTDIR\micontrol.exe" 0
    DetailPrint "Atalho criado no Ambiente de Trabalho."
  ${EndIf}

  ; ── Windows startup (user choice) ─────────────────────────────────────────
  ${If} $DoStartup == ${BST_CHECKED}
    WriteRegStr SHCTX "Software\Microsoft\Windows\CurrentVersion\Run" \
      "MiControl" '"$INSTDIR\micontrol.exe"'
    DetailPrint "MiControl configurado para iniciar com o Windows."
  ${EndIf}

  ; ── Hardware drivers ──────────────────────────────────────────────────────
  DetailPrint "Instalando drivers de hardware MiControl..."

  ; VirtualControlHID.sys — required for performance mode switching
  DetailPrint "  > VirtualControlHID.inf"
  nsExec::ExecToLog '"$SYSDIR\pnputil.exe" /add-driver "$INSTDIR\drivers\VirtualControlHID\virtualcontrolhid.inf" /install'
  Pop $0
  ${If} $0 = 0
    DetailPrint "  VirtualControlHID: instalado com sucesso."
  ${ElseIf} $0 = 3010
    DetailPrint "  VirtualControlHID: instalado — reinicialização necessária para ativar."
  ${Else}
    DetailPrint "  VirtualControlHID: pnputil retornou $0 (pode já estar atualizado)."
  ${EndIf}

  ; IoTDriver.sys + IoTService.exe — required for charging threshold control
  DetailPrint "  > iotdriver.inf"
  nsExec::ExecToLog '"$SYSDIR\pnputil.exe" /add-driver "$INSTDIR\drivers\IoTDriver\iotdriver.inf" /install'
  Pop $0
  ${If} $0 = 0
    DetailPrint "  IoTDriver: instalado com sucesso."
  ${ElseIf} $0 = 3010
    DetailPrint "  IoTDriver: instalado — reinicialização necessária para ativar."
  ${Else}
    DetailPrint "  IoTDriver: pnputil retornou $0 (pode já estar atualizado)."
  ${EndIf}

  ; Start IoTSvc if present (fails silently if already running)
  nsExec::ExecToLog '"$SYSDIR\sc.exe" start IoTSvc'
  Pop $0

  ; ── Scheduled task for elevated hardware operations (no UAC on use) ────────
  ; Registered via XML so we can set MultipleInstancesPolicy=StopExisting and
  ; ExecutionTimeLimit=PT30S, preventing the task from getting stuck in "Queued"
  ; state if a previous elevated helper is still running.
  ; Uses nsExec directly — works when installer is run elevated.
  ; If installer is not elevated, schtasks /create fails silently and the app's
  ; self-healing (ensure_task_correct_path) will fix it on first run via UAC.
  DetailPrint "Registando tarefa MiControlElevated..."
  WriteFile "$TEMP\MCElev.xml" '<?xml version="1.0" encoding="UTF-8"?><Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task"><Triggers><TimeTrigger><StartBoundary>2000-01-01T00:00:00</StartBoundary><Enabled>false</Enabled></TimeTrigger></Triggers><Principals><Principal id="Author"><LogonType>InteractiveToken</LogonType><RunLevel>HighestAvailable</RunLevel></Principal></Principals><Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><ExecutionTimeLimit>PT120S</ExecutionTimeLimit><Enabled>true</Enabled></Settings><Actions Context="Author"><Exec><Command>"$INSTDIR\micontrol.exe"</Command><Arguments>--elevated</Arguments></Exec></Actions></Task>'
  nsExec::ExecToLog '"$SYSDIR\schtasks.exe" /delete /tn "MiControlElevated" /f'
  Pop $0
  nsExec::ExecToLog '"$SYSDIR\schtasks.exe" /create /tn "MiControlElevated" /xml "$TEMP\MCElev.xml" /f'
  Pop $0
  Delete "$TEMP\MCElev.xml"
  DetailPrint "MiControlElevated task registered: $0"

  DetailPrint "Configuração de hardware concluída."
!macroend

!macro customUnInstall
  ; Remove desktop shortcut and startup entry (if they were created)
  Delete "$DESKTOP\MiControl.lnk"
  DeleteRegValue SHCTX "Software\Microsoft\Windows\CurrentVersion\Run" "MiControl"
  ; Also clean up old HKCU entry from previous currentUser installations
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "MiControl"

  ; Remove the elevated scheduled task
  nsExec::ExecToLog '"$SYSDIR\schtasks.exe" /delete /tn "MiControlElevated" /f'
  Pop $0
  DetailPrint "MiControlElevated task removed: $0"

  ; Stop and remove the IoTService Windows service
  nsExec::ExecToLog '"$SYSDIR\sc.exe" stop IoTSvc'
  Pop $0
  nsExec::ExecToLog '"$SYSDIR\sc.exe" delete IoTSvc'
  Pop $0
  DetailPrint "IoTSvc service removed: $0"

!macroend
