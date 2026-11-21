; Wrapper to compile-check installer-hooks.nsi modifications.
;
; Macros are only *parsed* when included; their bodies are compiled when they
; are expanded. Expanding POSTINSTALL/POSTUNINSTALL here keeps the check
; meaningful for code inside the hooks (${GetSize}, ${IfNot} ${Errors},
; nsExec quoting, ...) instead of only catching syntax errors at the top level.
Unicode true
!include LogicLib.nsh
!include FileFunc.nsh
!include "${NSISDIR}\Include\WinMessages.nsh"

Name "HookCompileCheck"
OutFile "$%TEMP%\hookcheck.exe"
InstallDir "$TEMP\hookcheck"

!include "installer-hooks.nsi"

Section "-dummy"
  !insertmacro NSIS_HOOK_PREINSTALL
  !insertmacro NSIS_HOOK_POSTINSTALL
  !insertmacro NSIS_HOOK_POSTUNINSTALL
SectionEnd