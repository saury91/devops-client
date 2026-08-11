!macro NSIS_HOOK_PREINSTALL
  DetailPrint "Stopping running devops-client to avoid file lock..."
  nsExec::Exec 'taskkill /F /IM "devops-client.exe" /T'
  Pop $0
  Sleep 800

  DetailPrint "Removing previous install files to avoid stale/read-only leftovers..."
  nsExec::Exec 'cmd /c rmdir /s /q "$INSTDIR"'
  Pop $0
  Sleep 300
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "Stopping running devops-client to avoid file lock..."
  nsExec::Exec 'taskkill /F /IM "devops-client.exe" /T'
  Pop $0
  Sleep 800
!macroend
