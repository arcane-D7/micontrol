; MiControl — custom NSIS installer hooks
; Adds:
;   • Startup-with-Windows registration (auto-start)
;   • Hardware driver installation (VirtualControlHID + IoTDriver)
;   • ecram_service deployment into the DriverStore
;   • MiControlElevated scheduled task + autonomous MiControlBridge service
;   • Face Unlock: MiControlFace auth service + micontrol_facecp.dll (CP)

; NOTE: Tauri v2 invokes `NSIS_HOOK_PREINSTALL` / `NSIS_HOOK_POSTINSTALL` /
;       `NSIS_HOOK_PREUNINSTALL` / `NSIS_HOOK_POSTUNINSTALL` from the
;       installer.nsi template.  (The Tauri v1 names `customHeader`,
;       `customPageBefore`, `customInstall`, `customUnInstall` are NOT called
;       by the custom template in this project — this was the reason the Face
;       Unlock service + drivers were silently skipped during install.)

!macro NSIS_HOOK_PREINSTALL
  ; Nothing to do before files are written.
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; ── Windows startup (register auto-start; desktop shortcut is handled by
  ;    the installer.nsi template's finish page) ─────────────────────────────
  WriteRegStr SHCTX "Software\Microsoft\Windows\CurrentVersion\Run" \
    "MiControl" '"$INSTDIR\micontrol.exe"'
  DetailPrint "MiControl set to start with Windows."

  ; ── Hardware drivers ──────────────────────────────────────────────────────
  DetailPrint "Installing MiControl hardware drivers..."

  ; VirtualControlHID.sys — required for performance mode switching
  DetailPrint "  > VirtualControlHID.inf"
  nsExec::ExecToLog '"$SYSDIR\pnputil.exe" /add-driver "$INSTDIR\drivers\VirtualControlHID\virtualcontrolhid.inf" /install'
  Pop $0
  ${If} $0 = 0
    DetailPrint "  VirtualControlHID: installed successfully."
  ${ElseIf} $0 = 3010
    DetailPrint "  VirtualControlHID: installed — reboot required to activate."
  ${ElseIf} $0 = 2
    DetailPrint "  VirtualControlHID: pnputil did not run (code $0) — check that the installer ran as Administrator."
  ${Else}
    DetailPrint "  VirtualControlHID: pnputil returned $0 — review required (reboot may be needed)."
  ${EndIf}

  ; IoTDriver.sys + IoTService.exe — required for charging threshold control
  DetailPrint "  > iotdriver.inf"
  nsExec::ExecToLog '"$SYSDIR\pnputil.exe" /add-driver "$INSTDIR\drivers\IoTDriver\iotdriver.inf" /install'
  Pop $0
  ${If} $0 = 0
    DetailPrint "  IoTDriver: installed successfully."
  ${ElseIf} $0 = 3010
    DetailPrint "  IoTDriver: installed — reboot required to activate."
  ${ElseIf} $0 = 2
    DetailPrint "  IoTDriver: pnputil did not run (code $0) — check that the installer ran as Administrator."
  ${Else}
    DetailPrint "  IoTDriver: pnputil returned $0 — review required (reboot may be needed)."
  ${EndIf}

  ; Start IoTSvc if present (fails silently if already running; 1056 benign)
  ; NOTE: intentionally REMOVED. On broken installs (bare binPath) this always
  ; printed a guaranteed "FAILED 2" noise line, and on healthy installs the
  ; deploy block below recreates the service anyway — so it added zero value.
  ; The IoTSvc is (re)created and started ONLY by `ecram_service.exe
  ; install-service` in the deploy block below.

  ; ── Deploy our ecram_service as the DriverStore IoTService.exe ─────────────
  ; The IoTDriver.sys security check requires a process named "IoTService.exe"
  ; located inside the driver's DriverStore FileRepository directory. We must
  ; replace that binary with our ecram_service.exe so EC RAM access works
  ; without the original Xiaomi IoTService (which rejects our IOCTLs).
  ;
  ; NOTE: `FindFirst` in NSIS returns a handle in $R0 and the FIRST match in
  ; $R1 — but a trailing `\IoTService.exe` wildcard yields NO match when the
  ; DriverStore package contains only `.sys`/`.inf` (the IoTService.exe may not
  ; exist). The previous code mis-tested `$R0 = 0` (a handle, never 0) and then
  ; proceeded with an EMPTY $R1 → CopyFiles failed + `sc create binPath=` failed
  ; with error 87 → IoTSvc was DELETED and never recreated. This rewrites the
  ; whole block with deterministic exe discovery + status-aware handling.
  DetailPrint "Deploying ecram_service into the DriverStore..."
  ; Locate the IoTDriver package dir deterministically.
  StrCpy $R0 ""
  FindFirst $R1 $R2 "$SYSDIR\DriverStore\FileRepository\iotdriver.inf_*"
  ${If} $R1 = ""
    DetailPrint "  Warning: iotdriver.inf_* package not found in DriverStore — ecram_service not deployed."
  ${Else}
    StrCpy $R0 "$SYSDIR\DriverStore\FileRepository\$R2"
    DetailPrint "  IoTDriver package in DriverStore: $R0"
    ; Stop the RUNNING IoTSvc (if any) BEFORE touching files. Guarded by a
    ; RUNNING check (not mere existence) to avoid "[SC] ControlService
    ; FAILED 1062" noise when the service is already stopped.
    nsExec::ExecToLog '"$SYSDIR\cmd.exe" /c ""$SYSDIR\sc.exe" query IoTSvc | "$SYSDIR\findstr.exe" /i "RUNNING" > NUL 2>&1"'
    Pop $R3
    ${If} $R3 = 0
      ; CRITICAL (upgrade race): the OLD IoTSvc has failure actions
      ; `restart/5000/...` — if the SCM auto-restarts it between our `sc stop`
      ; and the file copy, the process re-locks IoTService.exe and CopyFiles
      ; silently keeps the OLD (locked) binary. Disable crash auto-restart
      ; FIRST so a stopped service stays stopped while we replace the file.
      ; (Re-enabled by the fresh `sc failure` after the new create below.)
      nsExec::ExecToLog '"$SYSDIR\sc.exe" failure IoTSvc reset= 0 actions= ""'
      Pop $R3
      nsExec::ExecToLog '"$SYSDIR\sc.exe" stop IoTSvc'
      Pop $R3
      Sleep 2000
    ${EndIf}
    ; Kill any lingering process that holds a lock on the binary.
    nsExec::ExecToLog '"$SYSDIR\taskkill.exe" /F /IM IoTService.exe'
    Pop $R3
    nsExec::ExecToLog '"$SYSDIR\taskkill.exe" /F /IM ecram_service.exe'
    Pop $R3
    Sleep 1000
    ; Candidate destination: the package dir's IoTService.exe.
    StrCpy $R4 "$R0\IoTService.exe"
    ; Backup existing binary (once) if present — gives a rollback point.
    ${If} ${FileExists} "$R4"
      ${IfNot} ${FileExists} "$R4.bak"
        CopyFiles /SILENT "$R4" "$R4.bak"
      ${EndIf}
    ${EndIf}
    ; Replace it with ours.
    CopyFiles /SILENT "$INSTDIR\ecram_service.exe" "$R4"
    ; CopyFiles never returns an error code — verify the file landed AND that
    ; it is the file we just copied (not the locked old binary CopyFiles kept).
    ${IfNot} ${FileExists} "$R4"
      DetailPrint "  ERROR: failed to copy ecram_service.exe to $R4 (destination file still absent)."
      Abort "Could not replace IoTService.exe in the DriverStore. The IoTSvc service was NOT modified."
    ${EndIf}
    DetailPrint "  ecram_service deployed -> $R4"
    ; Now recreate the service pointing to the deployed binary (idempotent:
    ; the old service may have a stale binPath or be missing entirely).
    ; SCM DeleteService is ASYNC: the entry lingers until all open handles
    ; close and the service stops. A fixed Sleep(1000) is not guaranteed —
    ; `sc create` then fails with 1072 (marked for delete) or 1073 (exists).
    ; Wait until `sc query` reports the service is GONE (exit != 0) or timeout.
    nsExec::ExecToLog '"$SYSDIR\sc.exe" delete IoTSvc'
    Pop $R3
    StrCpy $R5 0           ; delete-confirm retry counter
    iot_delete_wait:
      nsExec::ExecToStack '"$SYSDIR\sc.exe" query IoTSvc'
      Pop $R3
      Pop $R6             ; drain output
      ${If} $R3 <> 0
        Goto iot_deleted  ; 1060 = service does not exist → gone
      ${EndIf}
      Sleep 500
      IntOp $R5 $R5 + 1
      ${If} $R5 < 20      ; up to ~10 s
        Goto iot_delete_wait
      ${EndIf}
      DetailPrint "  ERROR: IoTSvc was not removed from SCM (open handle / pending delete)."
      Abort "Failed to recreate the IoTSvc service: the old entry was not removed from SCM."
    iot_deleted:
    DetailPrint "  IoTSvc removed from SCM (confirmed)."
    ; CRITICAL: the IoTSvc service is created by the Rust binary
    ; (`ecram_service.exe install-service <path>`), NOT by `sc create` here.
    ; The SCM requires the binPath to be `"C:\...\IoTService.exe" service`
    ; with the QUOTES + `service` token embedded IN THE VALUE — that is the
    ; only form that reliably reaches RUNNING (verified empirically: a bare
    ; binPath makes `sc start` fail with error 2 even when the file exists).
    ; NSIS's ExecToLog passes the command line through CommandLineToArgvW,
    ; which strips outer quotes, so writing `binPath= "\"$R4\" service"` in
    ; NSIS stores a BARE path → StartService FAILED 2. std::process::Command
    ; (used by install-service) escapes the embedded quotes correctly.
    nsExec::ExecToLog '"$INSTDIR\ecram_service.exe" install-service "$R4"'
    Pop $R3
    ${If} $R3 = 0
      ; install-service already verified RUNNING — do a final confirm poll.
      StrCpy $R5 0         ; RUNNING-confirm retry counter
      iot_run_wait:
        nsExec::ExecToStack '"$SYSDIR\cmd.exe" /c ""$SYSDIR\sc.exe" query IoTSvc | "$SYSDIR\findstr.exe" /c:"RUNNING" > NUL 2>&1"'
        Pop $R3
        Pop $R6           ; drain output
        ${If} $R3 = 0
          Goto iot_running
        ${EndIf}
        Sleep 500
        IntOp $R5 $R5 + 1
        ${If} $R5 < 30
          Goto iot_run_wait
        ${EndIf}
        DetailPrint "  ERROR: IoTSvc did not reach RUNNING after install-service. Checking binPath..."
        nsExec::ExecToLog '"$SYSDIR\sc.exe" qc IoTSvc'
        Pop $R3
        Abort "Failed to start the IoTSvc service: not in RUNNING state. EC RAM access disabled."
      iot_running:
        DetailPrint "  IoTSvc recreated and RUNNING with ecram_service (verified)."
    ${Else}
      DetailPrint "  ERROR: ecram_service install-service returned $R3 — service NOT recreated."
      Abort "Failed to create the IoTSvc service (code $R3). EC RAM access could not be configured."
    ${EndIf}
  ${EndIf}
  FindClose $R1

  ; ── Scheduled task for elevated hardware operations (no UAC on use) ────────
  ; Registered via XML so we can set MultipleInstancesPolicy=StopExisting and
  ; ExecutionTimeLimit=PT30S, preventing the task from getting stuck in "Queued"
  ; state if a previous elevated helper is still running.
  ; Uses nsExec directly — works when installer is run elevated.
  ; If installer is not elevated, schtasks /create fails silently and the app's
  ; self-healing (ensure_task_correct_path) will fix it on first run via UAC.
  DetailPrint "Registering MiControlElevated scheduled task..."
  ; Write the task XML with native NSIS file commands (FileOpen/FileWrite/FileClose).
  ; NOTE: this installer template is built with `Unicode true`, so FileWrite
  ; emits UTF-16 LE with BOM. The previous `<?xml version="1.0" encoding="UTF-8"?>`
  ; prolog then conflicts with the actual encoding and schtasks fails with
  ; "(1,40): unable to switch the encoding". An XML document without prolog is
  ; still valid — schtasks auto-detects the BOM and parses it fine.
  FileOpen $0 "$TEMP\MCElev.xml" w
  FileWrite $0 '<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task"><Triggers><TimeTrigger><StartBoundary>2000-01-01T00:00:00</StartBoundary><Enabled>false</Enabled></TimeTrigger></Triggers><Principals><Principal id="Author"><LogonType>InteractiveToken</LogonType><RunLevel>HighestAvailable</RunLevel></Principal></Principals><Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><ExecutionTimeLimit>PT120S</ExecutionTimeLimit><Enabled>true</Enabled></Settings><Actions Context="Author"><Exec><Command>"$INSTDIR\micontrol.exe"</Command><Arguments>--elevated</Arguments></Exec></Actions></Task>'
  FileClose $0
  nsExec::ExecToLog '"$SYSDIR\schtasks.exe" /delete /tn "MiControlElevated" /f'
  Pop $0
  ; Validate the create actually succeeded — a failed create must not print
  ; a misleading "registered: <code>" as if it were OK. The app's self-heal
  ; (ensure_task_correct_path) is still the fallback if this fails.
  nsExec::ExecToLog '"$SYSDIR\schtasks.exe" /create /tn "MiControlElevated" /xml "$TEMP\MCElev.xml" /f'
  Pop $0
  Delete "$TEMP\MCElev.xml"
  ${If} $0 = 0
    DetailPrint "  MiControlElevated task registered: OK"
  ${Else}
    DetailPrint "  WARNING: schtasks /create MiControlElevated returned $0 — the app will self-heal on first run (ensure_task_correct_path)."
  ${EndIf}

  ; ── Autonomous elevated bridge service (MiControlBridge) ────────────────────
  ; Installed as a Windows service running as NT AUTHORITY\SYSTEM. Provides a
  ; named pipe (\\.\pipe\micontrol_bridge) for privileged commands WITHOUT any
  ; UAC prompt after installation. The main app prefers this path; the
  ; scheduled task above remains only as a fallback.
  DetailPrint "Installing MiControlBridge service (autonomous elevated bridge)..."
  nsExec::ExecToLog '"$INSTDIR\micontrol_bridge.exe" install'
  Pop $0
  ${If} $0 = 0
    DetailPrint "  MiControlBridge service installed: $0 (OK)"
  ${Else}
    DetailPrint "  ERROR: MiControlBridge service install failed with code $0"
    ; Abort cancels the install with a message — the user sees why instead
    ; of a "successful" install with services missing.
    Abort "Failed to install the MiControlBridge service (code $0). Make sure the installer was run as Administrator and try again."
  ${EndIf}

  ; ── Face Unlock (Windows Hello-style, RGB webcam) ──────────────────────────
  ; Optional: the auth service + Credential Provider + models. All guarded —
  ; if the files are missing from the bundle, the app still works (the Face
  ; Unlock tab will show the service as not installed).
  DetailPrint "Setting up Face Unlock..."
  ${If} ${FileExists} "$INSTDIR\micontrol_face_svc.exe"
    ; Cleanup the OLD service entry first (idempotent). The previous
    ; `install` path UPDATEs the existing service in place — if the old
    ; process is mid-shutdown (slow camera teardown) the SCM refuses the
    ; change_config with ERROR_SERVICE_MARKED_FOR_DELETE / ACCESS_DENIED,
    ; which previously Aborted the whole installer. Deleting the entry first
    ; (delete is async-safe — the SCM completes it once the handle closes)
    ; and then creating fresh avoids that class of failure entirely.
    DetailPrint "  Removing old MiControlFace service entry (if present)..."
    nsExec::ExecToLog '"$SYSDIR\sc.exe" delete MiControlFace'
    Pop $1
    ; Install the LocalSystem auth service (auto-start) as a FRESH service.
    nsExec::ExecToLog '"$INSTDIR\micontrol_face_svc.exe" install'
    Pop $0
    ${If} $0 = 0
      DetailPrint "  MiControlFace service install: $0 (OK)"
    ${Else}
      ; Face Unlock is OPTIONAL: a failed auth service must never block the
      ; core app installer (the app runs fine without it; the Face tab shows
      ; the service as unavailable). Log the failure, continue the install,
      ; and let the post-install verification report the real state.
      DetailPrint "  WARNING: MiControlFace service install returned $0 (Face Unlock will remain unavailable on this upgrade)."
    ${EndIf}
  ${Else}
    DetailPrint "  Note: micontrol_face_svc.exe not found in bundle — Face Unlock disabled."
  ${EndIf}
  ${If} ${FileExists} "$INSTDIR\micontrol_facecp.dll"
    ; Register the Face Unlock Credential Provider. This DLL is a COM
    ; class factory (DllGetClassObject/DllCanUnloadNow) WITHOUT
    ; DllRegisterServer — regsvr32 therefore always fails (error 4).
    ; Registration is done by writing the CLSID + Credential Providers
    ; registry keys (the standard explicit registration for custom CPs):
    ;   HKLM\SOFTWARE\Classes\CLSID\{E071A7CE-5D7F-4063-9A10-AE39AEC64EE8}
    ;     (default) = "MiControl Face Unlock"
    ;     InprocServer32 (default) = "$INSTDIR\micontrol_facecp.dll"
    ;                    ThreadingModel = "Apartment"
    ;   HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Authentication\
    ;       Credential Providers\{E071A7CE-5D7F-4063-9A10-AE39AEC64EE8}
    ;     (default) = "MiControl Face Unlock"
    DetailPrint "  Registering micontrol_facecp.dll as Credential Provider (registry)..."
    WriteRegStr HKLM "SOFTWARE\Classes\CLSID\{E071A7CE-5D7F-4063-9A10-AE39AEC64EE8}" "" "MiControl Face Unlock"
    WriteRegStr HKLM "SOFTWARE\Classes\CLSID\{E071A7CE-5D7F-4063-9A10-AE39AEC64EE8}\InprocServer32" "" "$INSTDIR\micontrol_facecp.dll"
    WriteRegStr HKLM "SOFTWARE\Classes\CLSID\{E071A7CE-5D7F-4063-9A10-AE39AEC64EE8}\InprocServer32" "ThreadingModel" "Apartment"
    WriteRegStr HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Authentication\Credential Providers\{E071A7CE-5D7F-4063-9A10-AE39AEC64EE8}" "" "MiControl Face Unlock"
    WriteRegDWORD HKLM "SOFTWARE\Classes\CLSID\{E071A7CE-5D7F-4063-9A10-AE39AEC64EE8}" "DisableModeless" 1
  ${EndIf}
  ; Ensure the face data directory exists (SYSTEM-writable).
  CreateDirectory "$PROGRAMDATA\MiControl\face"
  DetailPrint "Face Unlock configured."

  ; ── Post-install service verification ───────────────────────────────────────
  ; `micontrol_face_svc.exe install` exits 0 as soon as SCM accepts the create
  ; — but if the service binary then fails to start (e.g. the FrameServer crash
  ; we fixed, or a locked DLL), the user sees a "successful" install with a dead
  ; service. Verify with `sc query` piped through findstr INSIDE `cmd /c`:
  ;   • run OUTSIDE cmd (direct nsExec) breaks — nsExec has no pipe semantics
  ;     and sc prints its usage text ("Invalid Option").
  ;   • run INSIDE `cmd /c`, the pipe is handled by cmd and the exit code of
  ;     the last command (findstr) is propagated to nsExec reliably.
  ; findstr returns 0 when it finds "RUNNING", 1 when it does not.
  DetailPrint "Verifying installed services..."
  ; Bridge check: sc query must return 0 (service exists).
  nsExec::ExecToLog '"$SYSDIR\sc.exe" query MiControlBridge'
  Pop $0
  ${If} $0 <> 0
    DetailPrint "  WARNING: MiControlBridge did not respond to sc (code $0)."
  ${Else}
    DetailPrint "  MiControlBridge present (sc query OK)."
  ${EndIf}
  ${If} ${FileExists} "$INSTDIR\micontrol_face_svc.exe"
    ; Give the service a moment to finish starting (models load at boot).
    Sleep 2500
    nsExec::ExecToLog '"$SYSDIR\cmd.exe" /c ""$SYSDIR\sc.exe" query MiControlFace | "$SYSDIR\findstr.exe" /i "RUNNING" > NUL 2>&1"'
    Pop $0
    ${If} $0 = 0
      DetailPrint "  MiControlFace is RUNNING."
    ${Else}
      DetailPrint "  WARNING: MiControlFace is not RUNNING after installation. Check logs at C:\ProgramData\MiControl\face\face_svc.log"
    ${EndIf}
  ${EndIf}
  DetailPrint "Service verification complete."

  DetailPrint "Hardware configuration complete."
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
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
  ${If} $0 = 0
    DetailPrint "MiControlBridge service removed: $0 (OK)"
  ${Else}
    DetailPrint "WARNING: MiControlBridge uninstall returned $0 (ignored if already removed)."
  ${EndIf}

  ; Remove the Face Unlock auth service + Credential Provider
  ${If} ${FileExists} "$INSTDIR\micontrol_face_svc.exe"
    nsExec::ExecToLog '"$INSTDIR\micontrol_face_svc.exe" remove'
    Pop $0
    ${If} $0 = 0
      DetailPrint "MiControlFace service removed: $0 (OK)"
    ${Else}
      DetailPrint "WARNING: MiControlFace remove returned $0 (ignored if already removed)."
    ${EndIf}
  ${EndIf}
  ${If} ${FileExists} "$INSTDIR\micontrol_facecp.dll"
    ; Remove the Credential Provider registry keys (replaces regsvr32 /u,
    ; which failed because the DLL has no DllRegisterServer).
    DeleteRegKey HKLM "SOFTWARE\Classes\CLSID\{E071A7CE-5D7F-4063-9A10-AE39AEC64EE8}"
    DeleteRegKey HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Authentication\Credential Providers\{E071A7CE-5D7F-4063-9A10-AE39AEC64EE8}"
    DetailPrint "micontrol_facecp.dll Credential Provider unregistered"
  ${EndIf}

  ; Stop and remove the IoTService Windows service.  Only attempt delete if
  ; the service actually exists (avoids misleading "deleted" messages when it
  ; was never created — the app's self-healing path creates it on demand too).
  ; pipe inside cmd /c (not nsExec) so findstr's exit code is reliable.
  nsExec::ExecToLog '"$SYSDIR\cmd.exe" /c ""$SYSDIR\sc.exe" query IoTSvc | "$SYSDIR\findstr.exe" /c:"SERVICE_NAME" > NUL 2>&1"'
  Pop $0
  ${If} $0 = 0
    nsExec::ExecToLog '"$SYSDIR\sc.exe" stop IoTSvc'
    Pop $0
    Sleep 2000
    nsExec::ExecToLog '"$SYSDIR\sc.exe" delete IoTSvc'
    Pop $0
    DetailPrint "IoTSvc service removed: $0"
  ${Else}
    DetailPrint "IoTSvc service not present — skipping removal."
  ${EndIf}

!macroend
