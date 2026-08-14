; Wrapper to compile-check installer-hooks.nsi modifications
Unicode true
!include LogicLib.nsh
!include FileFunc.nsh
!include "${NSISDIR}\Include\WinMessages.nsh"

Name "HookCompileCheck"
OutFile "$%TEMP%\hookcheck.exe"
InstallDir "$TEMP\hookcheck"

Section "-dummy"
SectionEnd

!include "installer-hooks.nsi"
