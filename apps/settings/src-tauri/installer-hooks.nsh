; Hece installer hooks (Tauri NSIS). The bundle's main binary is the settings window; these hooks add
; what makes Hece itself work: the "Hece" shortcut starts the background app (hecew.exe), autostart,
; stopping running copies before files are replaced, and a full cleanup when the user asks for it.

; Stops this user's running Hece processes (another signed-in user's copies are theirs).
!macro HECE_STOP_PROCESSES
  ReadEnvStr $R9 USERNAME
  nsExec::Exec `"$SYSDIR\taskkill.exe" /F /FI "USERNAME eq $R9" /IM hecew.exe /IM hece.exe /IM hece-engine.exe`
  Pop $R8
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro HECE_STOP_PROCESSES
  ; Earlier builds were installed by a script to %LOCALAPPDATA%\Programs\Hece; remove that copy so
  ; only one Hece exists.
  ${If} ${FileExists} "$LOCALAPPDATA\Programs\Hece\hecew.exe"
    RMDir /r "$LOCALAPPDATA\Programs\Hece"
    Delete "$SMPROGRAMS\Hece Ayarlar.lnk"
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Tauri's own "Hece" shortcut opens the main binary (settings); "Hece" should start the app.
  CreateShortCut "$SMPROGRAMS\Hece.lnk" "$INSTDIR\hecew.exe" "" "$INSTDIR\hece-settings.exe" 0 SW_SHOWNORMAL "" "Hece voice dictation"
  CreateShortCut "$SMPROGRAMS\Hece Settings.lnk" "$INSTDIR\hece-settings.exe" "" "$INSTDIR\hece-settings.exe" 0 SW_SHOWNORMAL "" "Hece settings"
  ; Silent and passive installs have already made a desktop shortcut to the settings; point it at Hece.
  ${If} ${FileExists} "$DESKTOP\Hece.lnk"
    CreateShortCut "$DESKTOP\Hece.lnk" "$INSTDIR\hecew.exe" "" "$INSTDIR\hece-settings.exe" 0 SW_SHOWNORMAL "" "Hece voice dictation"
  ${EndIf}
  ; Updates keep the user's autostart choice.
  ${If} $UpdateMode <> 1
    nsExec::Exec `"$INSTDIR\hece.exe" autostart on`
    Pop $R8
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro HECE_STOP_PROCESSES
  ${If} $UpdateMode <> 1
    Delete "$SMPROGRAMS\Hece.lnk"
    Delete "$SMPROGRAMS\Hece Settings.lnk"
    Delete "$DESKTOP\Hece.lnk"
    ${If} $DeleteAppDataCheckboxState = 1
      ; API keys live in the Credential Manager as "<provider>.hece".
      nsExec::Exec `powershell -NoProfile -ExecutionPolicy Bypass -Command "cmdkey /list | Select-String 'target=(\S+\.hece)$$' | ForEach-Object { cmdkey /delete:$$($$_.Matches[0].Groups[1].Value) }"`
      Pop $R8
    ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    ; config.toml, then models, CUDA runtime, history, logs and the API token.
    RMDir /r "$APPDATA\Hece"
    RMDir /r "$LOCALAPPDATA\hece"
  ${EndIf}
!macroend
