; Rustshot NSIS installer (per-user, no admin required).
; Build: makensis packaging\installer.nsi   (optionally /DPRODUCT_VERSION=x.y.z)
; Output: dist\rustshot-<version>-setup.exe
; In-app updates run it as "setup.exe /S /RELAUNCH" (see .onInit and
; .onInstSuccess); interactive installs are unaffected by that path.
!ifndef PRODUCT_VERSION
  !define PRODUCT_VERSION "0.6.3"
!endif
!define PRODUCT_NAME "rustshot"
!define PRODUCT_DISPLAY "Rustshot"
!define PRODUCT_EXE "rustshot.exe"
!define PRODUCT_PUBLISHER "PowerNapps"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_NAME}"

Unicode true
ManifestDPIAware true
SetCompressor /SOLID lzma

!include "LogicLib.nsh"
!include "FileFunc.nsh"
!include "Sections.nsh"

Name "${PRODUCT_DISPLAY} ${PRODUCT_VERSION}"
Icon "icons\rustshot.ico"
UninstallIcon "icons\rustshot.ico"
BrandingText "${PRODUCT_DISPLAY} ${PRODUCT_VERSION}"
OutFile "..\dist\${PRODUCT_NAME}-${PRODUCT_VERSION}-setup.exe"
InstallDir "$LOCALAPPDATA\${PRODUCT_NAME}"
InstallDirRegKey HKCU "Software\${PRODUCT_NAME}" "InstallDir"
RequestExecutionLevel user

; Every Start menu / desktop shortcut any version made, the old lowercase
; names included (Delete and RMDir ignore case; both spellings are listed
; so the intent is plain). RMDir only removes an empty folder.
!macro RemoveShortcuts
  Delete "$SMPROGRAMS\${PRODUCT_NAME}\${PRODUCT_NAME}.lnk"
  Delete "$SMPROGRAMS\${PRODUCT_NAME}\Uninstall.lnk"
  Delete "$SMPROGRAMS\${PRODUCT_DISPLAY}\${PRODUCT_DISPLAY}.lnk"
  Delete "$SMPROGRAMS\${PRODUCT_DISPLAY}\${PRODUCT_DISPLAY} Settings.lnk"
  Delete "$SMPROGRAMS\${PRODUCT_DISPLAY}\Uninstall.lnk"
  RMDir "$SMPROGRAMS\${PRODUCT_NAME}"
  RMDir "$SMPROGRAMS\${PRODUCT_DISPLAY}"
  Delete "$DESKTOP\${PRODUCT_NAME}.lnk"
  Delete "$DESKTOP\${PRODUCT_DISPLAY}.lnk"
!macroend

; A lowercase "rustshot" folder still there holds files of the user's own
; (RMDir left it): renamed to "Rustshot" (a case-only rename keeps every
; file), so the shortcuts land in a folder Start shows capitalized.
!macro CapitalizeStartFolder
  ${If} ${FileExists} "$SMPROGRAMS\${PRODUCT_NAME}\*.*"
    Rename "$SMPROGRAMS\${PRODUCT_NAME}" "$SMPROGRAMS\${PRODUCT_DISPLAY}"
  ${EndIf}
!macroend

Page directory
Page components
Page instfiles
UninstPage uninstConfirm
UninstPage components
UninstPage instfiles

Section "${PRODUCT_DISPLAY} (required)" SecMain
  SectionIn RO
  SetOutPath "$INSTDIR"
  ${If} ${Silent}
    ; Silent update: move the old exe aside (allowed while it runs: the old
    ; daemon after the wait in .onInit, or a Settings window that started
    ; this update with the tray icon off) so File succeeds. The next start
    ; deletes rustshot.exe.old.
    Delete "$INSTDIR\${PRODUCT_EXE}.old"
    ${If} ${FileExists} "$INSTDIR\${PRODUCT_EXE}"
      Rename "$INSTDIR\${PRODUCT_EXE}" "$INSTDIR\${PRODUCT_EXE}.old"
    ${EndIf}
  ${EndIf}
  File "..\target\release\${PRODUCT_EXE}"
  File "..\LICENSE"
  File "..\THIRD_PARTY_NOTICES.md"
  File "..\NOTICE"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr HKCU "Software\${PRODUCT_NAME}" "InstallDir" "$INSTDIR"
  ; Start menu. 0.1.x made "rustshot\rustshot.lnk" (lowercase, no icon);
  ; Windows paths ignore case, so writing "Rustshot\Rustshot.lnk" over it
  ; would keep the old names (Start shows "rustshot"). Remove the old
  ; shortcuts and folder first, then make them fresh, the icon explicit.
  !insertmacro RemoveShortcuts
  !insertmacro CapitalizeStartFolder
  CreateDirectory "$SMPROGRAMS\${PRODUCT_DISPLAY}"
  CreateShortcut "$SMPROGRAMS\${PRODUCT_DISPLAY}\${PRODUCT_DISPLAY}.lnk" "$INSTDIR\${PRODUCT_EXE}" "" "$INSTDIR\${PRODUCT_EXE}" 0
  ; The way to Settings with the tray icon off (no background process).
  CreateShortcut "$SMPROGRAMS\${PRODUCT_DISPLAY}\${PRODUCT_DISPLAY} Settings.lnk" "$INSTDIR\${PRODUCT_EXE}" "settings" "$INSTDIR\${PRODUCT_EXE}" 0
  CreateShortcut "$SMPROGRAMS\${PRODUCT_DISPLAY}\Uninstall.lnk" "$INSTDIR\uninstall.exe" "" "$INSTDIR\uninstall.exe" 0
  ; Settings > Apps > Installed apps. The key name stays "rustshot", so an
  ; install or update over any older version (the in-app update runs this
  ; section too) rewrites every value here.
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "${PRODUCT_DISPLAY}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${PRODUCT_VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "${PRODUCT_PUBLISHER}"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\${PRODUCT_EXE},0"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "URLInfoAbout" "https://powernapps.net/r/rustshot"
  WriteRegStr HKCU "${UNINST_KEY}" "HelpLink" "https://github.com/nappsllc/rustshot/issues"
  WriteRegStr HKCU "${UNINST_KEY}" "URLUpdateInfo" "https://github.com/nappsllc/rustshot/releases"
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
  ; What is installed, in KB, measured once the files above are written.
  ${GetSize} "$INSTDIR" "/S=0K /G=0" $0 $1 $2
  WriteRegDWORD HKCU "${UNINST_KEY}" "EstimatedSize" $0
SectionEnd

; Ticked (by default; a silent update keeps it only where a desktop
; shortcut, any case, existed: .onInit): made fresh with the new name.
; Unticked: RemoveShortcuts took the old one away.
Section "Desktop shortcut" SecDesktop
  CreateShortcut "$DESKTOP\${PRODUCT_DISPLAY}.lnk" "$INSTDIR\${PRODUCT_EXE}" "" "$INSTDIR\${PRODUCT_EXE}" 0
SectionEnd

; An existing value is kept as it is (it may say "daemon --force", which
; runs with the tray icon off too); only a missing one is written.
Section "Start daemon with Windows" SecStartup
  ClearErrors
  ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${PRODUCT_NAME}"
  ${If} ${Errors}
  ${OrIf} $0 == ""
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${PRODUCT_NAME}" \
        '"$INSTDIR\${PRODUCT_EXE}" daemon'
  ${EndIf}
SectionEnd

; Silent installs only; interactive installs are unchanged.
Function .onInit
  ${If} ${Silent}
    ; An update keeps the user's choices: over an existing install the
    ; optional sections run again only if their shortcut / Run value exists.
    ReadRegStr $0 HKCU "Software\${PRODUCT_NAME}" "InstallDir"
    ${If} $0 != ""
      ${IfNot} ${FileExists} "$DESKTOP\${PRODUCT_DISPLAY}.lnk"
        !insertmacro UnselectSection ${SecDesktop}
      ${EndIf}
      ClearErrors
      ReadRegStr $1 HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${PRODUCT_NAME}"
      ${If} ${Errors}
        !insertmacro UnselectSection ${SecStartup}
      ${EndIf}
    ${EndIf}
    ; Wait up to 10 s for the running daemon (its hidden "rustshot_tray"
    ; window) to exit.
    StrCpy $2 0
    ${Do}
      FindWindow $0 "rustshot_tray"
      ${If} $0 = 0
        ${Break}
      ${EndIf}
      Sleep 100
      IntOp $2 $2 + 1
    ${LoopUntil} $2 >= 100
  ${EndIf}
FunctionEnd

; "/S /RELAUNCH": start the new daemon after a successful silent install.
; "--after-update" makes it wait for (never signal) a still-running old
; daemon, then exit quietly if that one never goes away.
Function .onInstSuccess
  ${If} ${Silent}
    ${GetParameters} $0
    ClearErrors
    ${GetOptions} $0 "/RELAUNCH" $1
    ${IfNot} ${Errors}
      ; "/FORCE": the daemon that updated ran with --force (the tray icon
      ; off but kept running); the new one does too.
      ClearErrors
      ${GetOptions} $0 "/FORCE" $1
      ${If} ${Errors}
        Exec '"$INSTDIR\${PRODUCT_EXE}" daemon --after-update'
      ${Else}
        Exec '"$INSTDIR\${PRODUCT_EXE}" daemon --after-update --force'
      ${EndIf}
    ${EndIf}
  ${EndIf}
FunctionEnd

Section "-un.Main"
  ; Win+Shift+S back to the Snipping Tool (also drops the user's choice of
  ; Rustshot and restores a RegisteredApplications value it replaced); our
  ; ProgID again in case the exe could not run (Software\rustshot, with
  ; the Capabilities, goes below).
  ExecWait '"$INSTDIR\${PRODUCT_EXE}" register-screenclip off'
  DeleteRegKey HKCU "Software\Classes\Rustshot.ScreenClip"
  Delete "$INSTDIR\${PRODUCT_EXE}"
  Delete "$INSTDIR\${PRODUCT_EXE}.old"
  Delete "$INSTDIR\uninstall.exe"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\THIRD_PARTY_NOTICES.md"
  Delete "$INSTDIR\NOTICE"
  ; In-app update downloads (files only; see update_install::update_dir).
  Delete "$LOCALAPPDATA\${PRODUCT_NAME}\update\*"
  RMDir "$LOCALAPPDATA\${PRODUCT_NAME}\update"
  RMDir "$INSTDIR"
  !insertmacro RemoveShortcuts
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${PRODUCT_NAME}"
  DeleteRegKey HKCU "${UNINST_KEY}"
  DeleteRegKey HKCU "Software\${PRODUCT_NAME}"
SectionEnd

; Off unless ticked, so never in a silent uninstall: the capture history
; (history::find_root) is the user's data; the same folder keeps the
; downloaded blur model and language files (src/datadir.rs), kept with it
; (a reinstall uses them again). Saved files are not touched.
Section /o "un.Also remove capture history and downloaded files" SecUnHistory
  RMDir /r "$LOCALAPPDATA\${PRODUCT_NAME}-data\history"
  RMDir /r "$LOCALAPPDATA\${PRODUCT_NAME}-data\models"
  RMDir /r "$LOCALAPPDATA\${PRODUCT_NAME}-data\lang"
  RMDir "$LOCALAPPDATA\${PRODUCT_NAME}-data"
SectionEnd
