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

  ; ── Deploy our ecram_service as the DriverStore IoTService.exe ─────────────
  ; The IoTDriver.sys security check requires a process named "IoTService.exe"
  ; located inside the driver's DriverStore FileRepository directory. We must
  ; replace that binary with our ecram_service.exe so EC RAM access works
  ; without the original Xiaomi IoTService (which rejects our IOCTLs).
  DetailPrint "Configurando ecram_service no DriverStore..."
  FindFirst $R0 $R1 "$SYSDIR\DriverStore\FileRepository\iotdriver.inf_*\IoTService.exe"
  ${If} $R0 = 0
    ; $R1 contains the found file path. Use it.
    nsExec::ExecToLog '"$SYSDIR\sc.exe" stop IoTSvc'
    Pop $R2
    Sleep 2000
    ; Backup the existing IoTService.exe (only if no backup yet)
    ${IfNot} ${FileExists} "$R1.bak"
      CopyFiles /SILENT "$R1" "$R1.bak"
    ${EndIf}
    ; Replace with our ecram_service.exe
    CopyFiles /SILENT "$INSTDIR\ecram_service.exe" "$R1"
    DetailPrint "  ecram_service deployed como IoTService.exe"
    ; Recreate the service pointing to the DriverStore exe
    nsExec::ExecToLog '"$SYSDIR\sc.exe" stop IoTSvc'
    Pop $R2
    nsExec::ExecToLog '"$SYSDIR\sc.exe" delete IoTSvc'
    Pop $R2
    Sleep 1000
    nsExec::ExecToLog '"$SYSDIR\sc.exe" create IoTSvc binPath= "$R1" service start= auto DisplayName= "MiControl IoT Bridge Service"'
    Pop $R2
    nsExec::ExecToLog '"$SYSDIR\sc.exe" config IoTSvc obj= LocalSystem'
    Pop $R2
    nsExec::ExecToLog '"$SYSDIR\sc.exe" start IoTSvc'
    Pop $R2
    DetailPrint "  IoTSvc reiniciado com ecram_service."
  ${Else}
    DetailPrint "  Aviso: DriverStore do IoTDriver não encontrado — ecram_service não deployado."
  ${EndIf}
  FindClose $R0

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

  ; ── Autonomous elevated bridge service (MiControlBridge) ────────────────────
  ; Installed as a Windows service running as NT AUTHORITY\SYSTEM. Provides a
  ; named pipe (\\.\pipe\micontrol_bridge) for privileged commands WITHOUT any
  ; UAC prompt after installation. The main app prefers this path; the
  ; scheduled task above remains only as a fallback.
  DetailPrint "Instalando serviço MiControlBridge (bridge elevada autónoma)..."
  nsExec::ExecToLog '"$INSTDIR\micontrol_bridge.exe" install'
  Pop $0
  DetailPrint "MiControlBridge service install: $0"

  ; ── Face Unlock (Windows Hello-style, RGB webcam) ──────────────────────────
  ; Optional: the auth service + Credential Provider + models. All guarded —
  ; if the files are missing from the bundle, the app still works (the Face
  ; Unlock tab will show the service as not installed).
  DetailPrint "Configurando Face Unlock..."
  ${If} ${FileExists} "$INSTDIR\micontrol_face_svc.exe"
    ; Install the LocalSystem auth service (auto-start).
    nsExec::ExecToLog '"$INSTDIR\micontrol_face_svc.exe" install'
    Pop $0
    DetailPrint "  MiControlFace service install: $0"
  ${EndIf}
  ${If} ${FileExists} "$INSTDIR\micontrol_facecp.dll"
    ; Register the Credential Provider DLL (COM class + CP registration).
    DetailPrint "  Registando micontrol_facecp.dll como Credential Provider..."
    nsExec::ExecToLog '"$SYSDIR\regsvr32.exe" /s "$INSTDIR\micontrol_facecp.dll"'
    Pop $0
    DetailPrint "  regsvr32 returned: $0"
  ${EndIf}
  ; Ensure the face data directory exists (SYSTEM-writable).
  CreateDirectory "$PROGRAMDATA\MiControl\face"
  DetailPrint "Face Unlock configurado."

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

  ; Remove the autonomous bridge service (if present)
  nsExec::ExecToLog '"$INSTDIR\micontrol_bridge.exe" uninstall'
  Pop $0
  DetailPrint "MiControlBridge service removed: $0"

  ; Remove the Face Unlock auth service + Credential Provider
  ${If} ${FileExists} "$INSTDIR\micontrol_face_svc.exe"
    nsExec::ExecToLog '"$INSTDIR\micontrol_face_svc.exe" remove'
    Pop $0
    DetailPrint "MiControlFace service removed: $0"
  ${EndIf}
  ${If} ${FileExists} "$INSTDIR\micontrol_facecp.dll"
    nsExec::ExecToLog '"$SYSDIR\regsvr32.exe" /s /u "$INSTDIR\micontrol_facecp.dll"'
    Pop $0
    DetailPrint "micontrol_facecp.dll unregistered: $0"
  ${EndIf}

  ; Stop and remove the IoTService Windows service
  nsExec::ExecToLog '"$SYSDIR\sc.exe" stop IoTSvc'
  Pop $0
  nsExec::ExecToLog '"$SYSDIR\sc.exe" delete IoTSvc'
  Pop $0
  DetailPrint "IoTSvc service removed: $0"

!macroend
