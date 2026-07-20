; Tauri NSIS installer hooks (bundle > windows > nsis > installerHooks).
;
; POSTINSTALL: the generated APP_ASSOCIATE macro points the `.poid`
; DefaultIcon at the application binary's own icon ("exe,0") - the *program*
; icon. M07 requires the *document* icon to be visibly different, and the
; bundler already installs it at $INSTDIR\icons\poid-document.ico, so point
; the file class there and tell the shell to refresh its icon cache.
;
; POSTUNINSTALL: the app's first launch writes a per-user association repair
; under HKCU\Software\Classes (see src/association.rs). The uninstaller only
; removes what the installer wrote, so clean our per-user keys too - but only
; if `.poid` still points at our ProgID (never clobber another app's claim).

!macro NSIS_HOOK_POSTINSTALL
  WriteRegStr SHCTX "Software\Classes\POID Document\DefaultIcon" "" "$INSTDIR\icons\poid-document.ico"
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0x1000, p 0, p 0)'
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegKey HKCU "Software\Classes\POIDStudio.Document"
  ReadRegStr $0 HKCU "Software\Classes\.poid" ""
  StrCmp $0 "POIDStudio.Document" 0 +2
  DeleteRegKey HKCU "Software\Classes\.poid"
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0x1000, p 0, p 0)'
!macroend
