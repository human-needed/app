; SPDX-License-Identifier: FSL-1.1-ALv2
; Clean uninstall (R-05). Before the program is deleted, it removes what it left: the
; start-at-login entry, the cloud link's credential, the log, the web view's cache and, if
; "Delete the application data" is ticked, the settings (app/src/uninstall.rs). Not on updates.
!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode <> 1
    !insertmacro CheckIfAppIsRunning "$INSTDIR\${MAINBINARYNAME}.exe" "${PRODUCTNAME}"
    ${If} $DeleteAppDataCheckboxState = 1
      ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --uninstall --purge'
    ${Else}
      ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --uninstall'
    ${EndIf}
  ${EndIf}
!macroend
