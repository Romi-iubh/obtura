; Obtura for Windows: the installer (NSIS, Modern UI). Made by build-in-container.sh, which
; passes VERSION, SIZE_KB, FILES and UNFILES (the files to install and to remove), ICON and
; OUTFILE.
;
; Installs for the current user only (no administrator rights asked) in
; %LOCALAPPDATA%\Programs\Obtura. The uninstaller removes exactly the files installed and
; never the user's settings, library or screenshots (%APPDATA%\Obtura, %LOCALAPPDATA%\Obtura,
; Pictures\Screenshots).

; A 64-bit installer, like Obtura itself (Windows 10 and 11, 64-bit).
Target amd64-unicode
!include "MUI2.nsh"

Name "Obtura"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\Obtura"
InstallDirRegKey HKCU "Software\Obtura" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
BrandingText "Obtura ${VERSION}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "Obtura"
VIAddVersionKey "FileDescription" "Obtura installer"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "CompanyName" "Rosaire"
VIAddVersionKey "LegalCopyright" "2026 Rosaire. GNU AGPL 3.0 or later."

!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Obtura"
!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_ABORTWARNING
!define MUI_COMPONENTSPAGE_NODESC
!define MUI_WELCOMEPAGE_TEXT "Obtura takes screenshots, reads their text and QR codes, and lets you annotate and blur them.$\r$\n$\r$\nEverything stays on this computer: no account, no upload.$\r$\n$\r$\nObtura is installed for you only; no administrator rights are needed."
!define MUI_FINISHPAGE_RUN "$INSTDIR\bin\obtura.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Start Obtura"
!define MUI_FINISHPAGE_TEXT "Press Print Screen to capture a region, or click Obtura's icon by the clock.$\r$\n$\r$\nIn Windows 11, Print Screen may open Windows' Snipping Tool: Obtura shows how to change that."

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

; A running Obtura (in the background, for Print Screen) holds its files: close it first.
!macro CloseObtura
  FindWindow $0 "ObturaMessages"
  StrCmp $0 0 +5
    DetailPrint "Closing the running Obtura"
    nsExec::Exec 'taskkill /IM obtura.exe /F'
    Pop $1
    Sleep 800
!macroend

Section "Obtura" SecMain
  SectionIn RO
  !insertmacro CloseObtura
  ; The files of an earlier version go first (exactly those, if it left its list).
  IfFileExists "$INSTDIR\uninstall.exe" 0 +2
    ExecWait '"$INSTDIR\uninstall.exe" /S _?=$INSTDIR'
  !include "${FILES}"
  SetOutPath "$INSTDIR"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  CreateShortcut "$SMPROGRAMS\Obtura.lnk" "$INSTDIR\bin\obtura.exe" "" "$INSTDIR\bin\obtura.exe" 0 SW_SHOWNORMAL "" "Screenshots, text reading and annotation"
  WriteRegStr HKCU "Software\Obtura" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "Obtura"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "Rosaire"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\bin\obtura.exe"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegStr HKCU "${UNINST_KEY}" "URLInfoAbout" "https://github.com/Romi-iubh/obtura"
  WriteRegDWORD HKCU "${UNINST_KEY}" "EstimatedSize" ${SIZE_KB}
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
SectionEnd

Section "Start Obtura in the background when I sign in (for Print Screen)" SecStartup
  CreateShortcut "$SMSTARTUP\Obtura.lnk" "$INSTDIR\bin\obtura.exe" "--gapplication-service" "$INSTDIR\bin\obtura.exe" 0 SW_SHOWNORMAL
SectionEnd

Section /o "A shortcut on the desktop" SecDesktop
  CreateShortcut "$DESKTOP\Obtura.lnk" "$INSTDIR\bin\obtura.exe" "" "$INSTDIR\bin\obtura.exe" 0
SectionEnd

Section "Uninstall"
  !insertmacro CloseObtura
  !include "${UNFILES}"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\Obtura.lnk"
  Delete "$SMSTARTUP\Obtura.lnk"
  Delete "$DESKTOP\Obtura.lnk"
  DeleteRegKey HKCU "${UNINST_KEY}"
  DeleteRegKey HKCU "Software\Obtura"
SectionEnd
