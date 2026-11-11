; RepoStacks per-user installer.
; Built by the release workflow:
;   makensis /DVERSION=0.1.0 /DSTAGE=<staging dir> /DOUTDIR=<output dir> scripts\installer.nsi
; The staging dir holds RepoStacks.exe (the window) and repostacks-cli.exe
; (the CLI — the -cli suffix matters: on Windows the two names would
; otherwise differ only by letter case and collide in one folder).
Unicode true
SetCompressor /SOLID lzma

!ifndef VERSION
  !define VERSION "0.1.0"
!endif
!ifndef STAGE
  !define STAGE "..\dist"
!endif
!ifndef OUTDIR
  !define OUTDIR "..\dist"
!endif

Name "RepoStacks"
OutFile "${OUTDIR}\RepoStacks-v${VERSION}-x64-Setup.exe"
InstallDir "$LOCALAPPDATA\RepoStacks"
RequestExecutionLevel user
ShowInstDetails show

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "RepoStacks"
VIAddVersionKey "FileDescription" "RepoStacks — your GitHub library for Windows"
VIAddVersionKey "FileVersion" "${VERSION}.0"
VIAddVersionKey "ProductVersion" "${VERSION}"

Section "RepoStacks" SecMain
  SetOutPath "$INSTDIR"
  File "${STAGE}\RepoStacks.exe"
  File "${STAGE}\repostacks-cli.exe"
  File "${STAGE}\LICENSE"

  CreateDirectory "$SMPROGRAMS\RepoStacks"
  CreateShortcut "$SMPROGRAMS\RepoStacks\RepoStacks.lnk" "$INSTDIR\RepoStacks.exe"
  CreateShortcut "$SMPROGRAMS\RepoStacks\Uninstall RepoStacks.lnk" "$INSTDIR\Uninstall.exe"

  WriteUninstaller "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\RepoStacks" "DisplayName" "RepoStacks"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\RepoStacks" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\RepoStacks" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\RepoStacks" "UninstallString" "$INSTDIR\Uninstall.exe"
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\RepoStacks" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\RepoStacks" "NoRepair" 1
SectionEnd

Section "Uninstall"
  Delete "$INSTDIR\RepoStacks.exe"
  Delete "$INSTDIR\repostacks-cli.exe"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\Uninstall.exe"
  ; The data folder (catalogue, settings, downloads beside the exe) belongs
  ; to the user; it is left in place on uninstall.
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\RepoStacks\RepoStacks.lnk"
  Delete "$SMPROGRAMS\RepoStacks\Uninstall RepoStacks.lnk"
  RMDir "$SMPROGRAMS\RepoStacks"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\RepoStacks"
SectionEnd
