; Based on cargo-packager 0.11.8's default NSIS template. Keep changes narrow:
; exact autostart cleanup and opt-in firewall cleanup during uninstall.
; Set the compression algorithm.
!if "{{compression}}" == ""
  SetCompressor /SOLID lzma
!else
  SetCompressor /SOLID "{{compression}}"
!endif

Unicode true

!include MUI2.nsh
!include FileFunc.nsh
!include x64.nsh
!include WinVer.nsh
!include WordFunc.nsh
!include "FileAssociation.nsh"
!include "StrFunc.nsh"
!include "StrFunc.nsh"
${StrCase}
${StrLoc}

!define MANUFACTURER "{{manufacturer}}"
!define PRODUCTNAME "{{product_name}}"
!define VERSION "{{version}}"
!define VERSIONWITHBUILD "{{version_with_build}}"
!define SHORTDESCRIPTION "{{short_description}}"
!define INSTALLMODE "{{install_mode}}"
!define LICENSE "{{license}}"
!define INSTALLERICON "{{installer_icon}}"
!define SIDEBARIMAGE "{{sidebar_image}}"
!define HEADERIMAGE "{{header_image}}"
!define MAINBINARYNAME "{{main_binary_name}}"
!define MAINBINARYSRCPATH "{{main_binary_path}}"
!define IDENTIFIER "{{identifier}}"
!define COPYRIGHT "{{copyright}}"
!define OUTFILE "{{out_file}}"
!define ARCH "{{arch}}"
!define PLUGINSPATH "{{additional_plugins_path}}"
!define ALLOWDOWNGRADES "{{allow_downgrades}}"
!define DISPLAYLANGUAGESELECTOR "{{display_language_selector}}"
!define UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCTNAME}"
!define MANUPRODUCTKEY "Software\${MANUFACTURER}\${PRODUCTNAME}"
!define UNINSTALLERSIGNCOMMAND "{{uninstaller_sign_cmd}}"
!define ESTIMATEDSIZE "{{estimated_size}}"
!define AUTOSTARTKEY "Software\Microsoft\Windows\CurrentVersion\Run"
!define AUTOSTARTVALUENAME "AirWiki"
!define FIREWALLHELPER "airwiki-windows-firewall-helper.exe"
!define VERSION_SENTINEL "__airwiki_invalid_semver__"
!define RELATION_NONE "none"
!define RELATION_NEWER "newer"
!define RELATION_SAME "same"
!define RELATION_OLDER "older"
!define NSIS_METADATA_ABSENT "absent"
!define NSIS_METADATA_COMPLETE "complete"
!define NSIS_METADATA_PARTIAL "partial"
!define UNINSTROOT "Software\Microsoft\Windows\CurrentVersion\Uninstall"

!if "${INSTALLMODE}" != "currentUser"
  !error "AirWiki 0.2.0 supports only currentUser Windows installs."
!endif
!if "${ALLOWDOWNGRADES}" != "false"
  !error "AirWiki 0.2.0 does not support Windows downgrades."
!endif

Name "${PRODUCTNAME}"
BrandingText "${COPYRIGHT}"
OutFile "${OUTFILE}"

VIProductVersion "${VERSIONWITHBUILD}"
VIAddVersionKey "ProductName" "${PRODUCTNAME}"
VIAddVersionKey "FileDescription" "${SHORTDESCRIPTION}"
VIAddVersionKey "LegalCopyright" "${COPYRIGHT}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"

; Plugins path, currently exists for linux only
!if "${PLUGINSPATH}" != ""
    !addplugindir "${PLUGINSPATH}"
!endif

!if "${UNINSTALLERSIGNCOMMAND}" != ""
  !uninstfinalize '${UNINSTALLERSIGNCOMMAND}'
!endif

; Handle install mode, `perUser`, `perMachine` or `both`
!if "${INSTALLMODE}" == "perMachine"
  RequestExecutionLevel highest
!endif

!if "${INSTALLMODE}" == "currentUser"
  RequestExecutionLevel user
!endif

!if "${INSTALLMODE}" == "both"
  !define MULTIUSER_MUI
  !define MULTIUSER_INSTALLMODE_INSTDIR "${PRODUCTNAME}"
  !define MULTIUSER_INSTALLMODE_COMMANDLINE
  !if "${ARCH}" == "x64"
    !define MULTIUSER_USE_PROGRAMFILES64
  !else if "${ARCH}" == "arm64"
    !define MULTIUSER_USE_PROGRAMFILES64
  !endif
  !define MULTIUSER_INSTALLMODE_DEFAULT_REGISTRY_KEY "${UNINSTKEY}"
  !define MULTIUSER_INSTALLMODE_DEFAULT_REGISTRY_VALUENAME "CurrentUser"
  !define MULTIUSER_INSTALLMODEPAGE_SHOWUSERNAME
  !define MULTIUSER_INSTALLMODE_FUNCTION RestorePreviousInstallLocation
  !define MULTIUSER_EXECUTIONLEVEL Highest
  !include MultiUser.nsh
!endif

; installer icon
!if "${INSTALLERICON}" != ""
  !define MUI_ICON "${INSTALLERICON}"
!endif

; installer sidebar image
!if "${SIDEBARIMAGE}" != ""
  !define MUI_WELCOMEFINISHPAGE_BITMAP "${SIDEBARIMAGE}"
!endif

; installer header image
!if "${HEADERIMAGE}" != ""
  !define MUI_HEADERIMAGE
  !define MUI_HEADERIMAGE_BITMAP  "${HEADERIMAGE}"
!endif

; Define registry key to store installer language
!define MUI_LANGDLL_REGISTRY_ROOT "HKCU"
!define MUI_LANGDLL_REGISTRY_KEY "${MANUPRODUCTKEY}"
!define MUI_LANGDLL_REGISTRY_VALUENAME "Installer Language"

; Installer pages, must be ordered as they appear
; 1. Welcome Page
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
!insertmacro MUI_PAGE_WELCOME

; 2. License Page (if defined)
!if "${LICENSE}" != ""
  !define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
  !insertmacro MUI_PAGE_LICENSE "${LICENSE}"
!endif

; 3. Install mode (if it is set to `both`)
!if "${INSTALLMODE}" == "both"
  !define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
  !insertmacro MULTIUSER_PAGE_INSTALLMODE
!endif


; 4. Custom page to ask user if he wants to reinstall/uninstall
;    only if a previous installtion was detected
Var ReinstallPageCheck
Page custom PageReinstall PageLeaveReinstall
Function PageReinstall
  ${If} $ExistingInstallKind == "none"
    Abort
  ${EndIf}
  StrCpy $R4 "$(older)"
  ${If} $ExistingInstallKind == "wix"
    StrCpy $R1 "$(olderOrUnknownVersionInstalled)"
    StrCpy $R2 "$(uninstallBeforeInstalling)"
    StrCpy $R3 "$(dontUninstall)"
    !insertmacro MUI_HEADER_TEXT "$(alreadyInstalled)" "$(choowHowToInstall)"
    StrCpy $R5 "wix"
    ; Default to no migration. The human must select the first radio explicitly.
    StrCpy $ReinstallPageCheck 2
  ${ElseIf} $InstallVersionRelation == "${RELATION_SAME}"
    StrCpy $R1 "$(alreadyInstalledLong)"
    StrCpy $R2 "$(addOrReinstall)"
    StrCpy $R3 "$(uninstallApp)"
    !insertmacro MUI_HEADER_TEXT "$(alreadyInstalled)" "$(chooseMaintenanceOption)"
    StrCpy $R5 "2"
  ${ElseIf} $InstallVersionRelation == "${RELATION_NEWER}"
    StrCpy $R1 "$(olderOrUnknownVersionInstalled)"
    StrCpy $R2 "$(uninstallBeforeInstalling)"
    StrCpy $R3 "$(dontUninstall)"
    !insertmacro MUI_HEADER_TEXT "$(alreadyInstalled)" "$(choowHowToInstall)"
    StrCpy $R5 "1"
  ${Else}
    SetErrorLevel 2
    Abort
  ${EndIf}

  Call SkipIfPassive

  nsDialogs::Create 1018
  Pop $R4
  ${IfThen} $(^RTL) == 1 ${|} nsDialogs::SetRTL $(^RTL) ${|}

  ${NSD_CreateLabel} 0 0 100% 24u $R1
  Pop $R1

  ${NSD_CreateRadioButton} 30u 50u -30u 8u $R2
  Pop $R2
  ${NSD_OnClick} $R2 PageReinstallUpdateSelection

  ${NSD_CreateRadioButton} 30u 70u -30u 8u $R3
  Pop $R3
  ${NSD_OnClick} $R3 PageReinstallUpdateSelection

  ; Check the first radio button if this the first time
  ; we enter this page or if the second button wasn't
  ; selected the last time we were on this page
  ${If} $ReinstallPageCheck != 2
    SendMessage $R2 ${BM_SETCHECK} ${BST_CHECKED} 0
  ${Else}
    SendMessage $R3 ${BM_SETCHECK} ${BST_CHECKED} 0
  ${EndIf}

  ${NSD_SetFocus} $R2
  nsDialogs::Show
FunctionEnd
Function PageReinstallUpdateSelection
  ${NSD_GetState} $R2 $R1
  ${If} $R1 == ${BST_CHECKED}
    StrCpy $ReinstallPageCheck 1
  ${Else}
    StrCpy $ReinstallPageCheck 2
  ${EndIf}
FunctionEnd
Function PageLeaveReinstall
  ${NSD_GetState} $R2 $R1

  ${If} $ExistingInstallKind == "wix"
    ${If} $R1 != ${BST_CHECKED}
      Abort
    ${EndIf}
    Goto reinst_uninstall
  ${EndIf}

  ; $R5 holds whether we are reinstalling the same version or not
  ; $R5 == "1" -> different versions
  ; $R5 == "2" -> same version
  ;
  ; $R1 holds the radio buttons state. its meaning is dependant on the context
  StrCmp $R5 "1" 0 +2 ; Existing install is not the same version?
    StrCmp $R1 "1" reinst_uninstall reinst_done ; $R1 == "1", then user chose to uninstall existing version, otherwise skip uninstalling
  StrCmp $R1 "1" reinst_done ; Same version? skip uninstalling

  reinst_uninstall:
    HideWindow
    ClearErrors

    ${If} $ExistingInstallKind == "wix"
      ReadRegStr $R1 HKLM "$ExistingUninstallKey" "UninstallString"
      ExecWait '$R1' $0
    ${Else}
      ReadRegStr $4 SHCTX "${MANUPRODUCTKEY}" ""
      ReadRegStr $R1 SHCTX "${UNINSTKEY}" "UninstallString"
      ExecWait '$R1 /P _?=$4' $0
    ${EndIf}

    BringToFront

    ${IfThen} ${Errors} ${|} StrCpy $0 2 ${|} ; ExecWait failed, set fake exit code

    ${If} $0 <> 0
    ${OrIf} ${FileExists} "$INSTDIR\${MAINBINARYNAME}.exe"
      ${If} $0 = 1 ; User aborted uninstaller?
        StrCmp $R5 "2" 0 +2 ; Is the existing install the same version?
          Quit ; ...yes, already installed, we are done
        Abort
      ${EndIf}
      MessageBox MB_ICONEXCLAMATION "$(unableToUninstall)"
      Abort
    ${Else}
      StrCpy $0 $R1 1
      ${IfThen} $0 == '"' ${|} StrCpy $R1 $R1 -1 1 ${|} ; Strip quotes from UninstallString
      Delete $R1
      RMDir $INSTDIR
    ${EndIf}
  reinst_done:
FunctionEnd

; 5. Choose install directoy page
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
!insertmacro MUI_PAGE_DIRECTORY

; 6. Start menu shortcut page
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
Var AppStartMenuFolder
!insertmacro MUI_PAGE_STARTMENU Application $AppStartMenuFolder

; 7. Installation page
!insertmacro MUI_PAGE_INSTFILES

; 8. Finish page
;
; Don't auto jump to finish page after installation page,
; because the installation page has useful info that can be used debug any issues with the installer.
!define MUI_FINISHPAGE_NOAUTOCLOSE
; Use show readme button in the finish page as a button create a desktop shortcut
!define MUI_FINISHPAGE_SHOWREADME
!define MUI_FINISHPAGE_SHOWREADME_TEXT "$(createDesktop)"
!define MUI_FINISHPAGE_SHOWREADME_FUNCTION CreateDesktopShortcut
; Show run app after installation.
!define MUI_FINISHPAGE_RUN "$INSTDIR\${MAINBINARYNAME}.exe"
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
!insertmacro MUI_PAGE_FINISH

; Uninstaller Pages
; 1. Confirm uninstall page
;
; Both optional cleanup choices are deliberately unchecked. Silent/passive
; uninstall therefore keeps user data and firewall rules.
Var RemoveFirewallCheckbox
Var RemoveFirewallCheckboxState
{{#if appdata_paths}}
Var DeleteAppDataCheckbox
Var DeleteAppDataCheckboxState
{{/if}}
!define /ifndef WS_EX_LAYOUTRTL         0x00400000
!define MUI_PAGE_CUSTOMFUNCTION_SHOW un.ConfirmShow
Function un.ConfirmShow
    FindWindow $1 "#32770" "" $HWNDPARENT ; Find inner dialog
    ${If} $(^RTL) == 1
      System::Call 'USER32::CreateWindowEx(i${__NSD_CheckBox_EXSTYLE}|${WS_EX_LAYOUTRTL},t"${__NSD_CheckBox_CLASS}",t "$(removeFirewallRules)",i${__NSD_CheckBox_STYLE},i 50,i 75,i 400, i 25,i$1,i0,i0,i0)i.s'
      {{#if appdata_paths}}
      System::Call 'USER32::CreateWindowEx(i${__NSD_CheckBox_EXSTYLE}|${WS_EX_LAYOUTRTL},t"${__NSD_CheckBox_CLASS}",t "$(deleteAppData)",i${__NSD_CheckBox_STYLE},i 50,i 100,i 400, i 25,i$1,i0,i0,i0)i.s'
      {{/if}}
    ${Else}
      System::Call 'USER32::CreateWindowEx(i${__NSD_CheckBox_EXSTYLE},t"${__NSD_CheckBox_CLASS}",t "$(removeFirewallRules)",i${__NSD_CheckBox_STYLE},i 0,i 75,i 400, i 25,i$1,i0,i0,i0)i.s'
      {{#if appdata_paths}}
      System::Call 'USER32::CreateWindowEx(i${__NSD_CheckBox_EXSTYLE},t"${__NSD_CheckBox_CLASS}",t "$(deleteAppData)",i${__NSD_CheckBox_STYLE},i 0,i 100,i 400, i 25,i$1,i0,i0,i0)i.s'
      {{/if}}
    ${EndIf}
    {{#if appdata_paths}}
    Pop $DeleteAppDataCheckbox
    {{/if}}
    Pop $RemoveFirewallCheckbox
    SendMessage $HWNDPARENT ${WM_GETFONT} 0 0 $1
    SendMessage $RemoveFirewallCheckbox ${WM_SETFONT} $1 1
    {{#if appdata_paths}}
    SendMessage $DeleteAppDataCheckbox ${WM_SETFONT} $1 1
    {{/if}}
FunctionEnd
!define MUI_PAGE_CUSTOMFUNCTION_LEAVE un.ConfirmLeave
Function un.ConfirmLeave
    SendMessage $RemoveFirewallCheckbox ${BM_GETCHECK} 0 0 $RemoveFirewallCheckboxState
    {{#if appdata_paths}}
    SendMessage $DeleteAppDataCheckbox ${BM_GETCHECK} 0 0 $DeleteAppDataCheckboxState
    {{/if}}
FunctionEnd
!insertmacro MUI_UNPAGE_CONFIRM

; 2. Uninstalling Page
!insertmacro MUI_UNPAGE_INSTFILES

;Languages
{{#each languages}}
!insertmacro MUI_LANGUAGE "{{this}}"
{{/each}}
LangString UnsupportedWindowsVersion ${LANG_ENGLISH} "AirWiki requires Windows 10 or Windows 11."
LangString UnsupportedWindowsVersion ${LANG_SPANISH} "AirWiki requiere Windows 10 o Windows 11."
LangString UnsupportedWindowsServer ${LANG_ENGLISH} "Windows Server is not supported. AirWiki requires Windows 10 or Windows 11 client."
LangString UnsupportedWindowsServer ${LANG_SPANISH} "Windows Server no es compatible. AirWiki requiere Windows 10 u 11 cliente."
LangString UnsupportedWindowsArchitecture ${LANG_ENGLISH} "AirWiki requires native x64 Windows on an AMD64 processor."
LangString UnsupportedWindowsArchitecture ${LANG_SPANISH} "AirWiki requiere Windows x64 nativo en un procesador AMD64."
!insertmacro MUI_RESERVEFILE_LANGDLL
{{#each language_files}}
  !include "{{this}}"
{{/each}}
LangString removeFirewallRules ${LANG_ENGLISH} "Remove AirWiki's restricted local-network firewall rules (administrator approval required)"
LangString removeFirewallRules ${LANG_SPANISH} "Quitar las reglas restringidas de red local de AirWiki (requiere aprobación de administrador)"
LangString firewallRulesRemain ${LANG_ENGLISH} "Windows could not remove the firewall rules. Uninstallation will continue and the rules will remain until removed from Windows Security."
LangString firewallRulesRemain ${LANG_SPANISH} "Windows no pudo quitar las reglas del firewall. La desinstalación continuará y las reglas permanecerán hasta quitarlas desde Seguridad de Windows."

!macro SetContext
  !if "${INSTALLMODE}" == "currentUser"
    SetShellVarContext current
  !else if "${INSTALLMODE}" == "perMachine"
    SetShellVarContext all
  !endif

  ${If} ${RunningX64}
    !if "${ARCH}" == "x64"
      SetRegView 64
    !else if "${ARCH}" == "arm64"
      SetRegView 64
    !else
      SetRegView 32
    !endif
  ${EndIf}
!macroend

Var ExistingInstallKind
Var ExistingUninstallKey
Var InstalledVersion
Var InstallVersionRelation
Var WixMetadataCount
Var WixCandidateKey
Var NsisMetadataState
Var SilentMode
Var PlatformRejectionMessage
Var PassiveMode
Var UpdaterMode

Function RejectUnsupportedPlatform
  IfSilent platform_reject_abort
  MessageBox MB_OK|MB_ICONSTOP "$PlatformRejectionMessage"
  platform_reject_abort:
    SetErrorLevel 2
    Abort
FunctionEnd

Function EnforceSupportedWindows
  ${IfNot} ${AtLeastWin10}
    StrCpy $PlatformRejectionMessage "$(UnsupportedWindowsVersion)"
    Call RejectUnsupportedPlatform
  ${EndIf}
  ${If} ${IsServerOS}
    StrCpy $PlatformRejectionMessage "$(UnsupportedWindowsServer)"
    Call RejectUnsupportedPlatform
  ${EndIf}
  ${IfNot} ${IsNativeAMD64}
    StrCpy $PlatformRejectionMessage "$(UnsupportedWindowsArchitecture)"
    Call RejectUnsupportedPlatform
  ${EndIf}
FunctionEnd

Function ClassifyExistingInstallation
  StrCpy $ExistingInstallKind "none"
  StrCpy $ExistingUninstallKey ""
  StrCpy $InstalledVersion ""
  StrCpy $InstallVersionRelation "${RELATION_NONE}"
  StrCpy $WixMetadataCount 0
  StrCpy $WixCandidateKey ""
  StrCpy $NsisMetadataState "${NSIS_METADATA_ABSENT}"

  ; Validate the signed candidate independently of registry state.
  nsis_tauri_utils::SemverCompare "${VERSION}" "${VERSION_SENTINEL}"
  Pop $0
  ${If} $0 != 1
    SetErrorLevel 2
    Abort
  ${EndIf}

  ; Scan every matching WiX entry. Never select the first match.
  StrCpy $0 0
  classify_wix_loop:
    EnumRegKey $1 HKLM "${UNINSTROOT}" $0
    StrCmp $1 "" classify_nsis_scan
    IntOp $0 $0 + 1
    ReadRegStr $2 HKLM "${UNINSTROOT}\$1" "DisplayName"
    ReadRegStr $3 HKLM "${UNINSTROOT}\$1" "Publisher"
    StrCmp "$2$3" "${PRODUCTNAME}${MANUFACTURER}" 0 classify_wix_loop
    IntOp $WixMetadataCount $WixMetadataCount + 1
    ReadRegStr $2 HKLM "${UNINSTROOT}\$1" "UninstallString"
    StrCmp $2 "" classify_reject
    ${StrCase} $3 $2 "L"
    ${StrLoc} $2 $3 "msiexec" ">"
    StrCmp $2 0 0 classify_reject
    ${If} $WixMetadataCount == 1
      StrCpy $WixCandidateKey "${UNINSTROOT}\$1"
      ReadRegStr $InstalledVersion HKLM "$WixCandidateKey" "DisplayVersion"
    ${EndIf}
    Goto classify_wix_loop

  ; Enumerate the parent so an existing-but-empty exact NSIS key is partial,
  ; not indistinguishable from an absent key.
  classify_nsis_scan:
    StrCpy $0 0
  classify_nsis_loop:
    EnumRegKey $1 SHCTX "${UNINSTROOT}" $0
    StrCmp $1 "" classify_evaluate
    IntOp $0 $0 + 1
    StrCmp $1 "${PRODUCTNAME}" 0 classify_nsis_loop
    StrCpy $NsisMetadataState "${NSIS_METADATA_PARTIAL}"
    ReadRegStr $2 SHCTX "${UNINSTKEY}" "DisplayName"
    ReadRegStr $3 SHCTX "${UNINSTKEY}" "Publisher"
    ReadRegStr $4 SHCTX "${UNINSTKEY}" "InstallLocation"
    ReadRegStr $5 SHCTX "${UNINSTKEY}" "UninstallString"
    ReadRegStr $6 SHCTX "${UNINSTKEY}" "DisplayVersion"
    StrCmp $2 "" classify_reject
    StrCmp $3 "" classify_reject
    StrCmp $4 "" classify_reject
    StrCmp $5 "" classify_reject
    StrCmp $6 "" classify_reject
    StrCpy $NsisMetadataState "${NSIS_METADATA_COMPLETE}"
    Goto classify_evaluate

  classify_evaluate:
    ${If} $WixMetadataCount > 1
      Goto classify_reject
    ${EndIf}
    ${If} $WixMetadataCount == 1
      ${If} $NsisMetadataState != "${NSIS_METADATA_ABSENT}"
        Goto classify_reject
      ${EndIf}
      StrCpy $ExistingInstallKind "wix"
      StrCpy $ExistingUninstallKey "$WixCandidateKey"
      Goto classify_validate
    ${EndIf}
    ${If} $NsisMetadataState == "${NSIS_METADATA_PARTIAL}"
      Goto classify_reject
    ${ElseIf} $NsisMetadataState == "${NSIS_METADATA_COMPLETE}"
      StrCpy $ExistingInstallKind "nsis"
      StrCpy $ExistingUninstallKey "${UNINSTKEY}"
      StrCpy $InstalledVersion $6
      Goto classify_validate
    ${EndIf}
    Goto classify_done

  classify_validate:
    StrCmp $InstalledVersion "" classify_reject
    nsis_tauri_utils::SemverCompare "$InstalledVersion" "${VERSION_SENTINEL}"
    Pop $0
    StrCmp $0 1 0 classify_reject
    nsis_tauri_utils::SemverCompare "${VERSION}" "$InstalledVersion"
    Pop $0
    StrCmp $0 1 classify_newer
    StrCmp $0 0 classify_same
    StrCmp $0 -1 classify_older classify_reject
  classify_newer:
    StrCpy $InstallVersionRelation "${RELATION_NEWER}"
    Goto classify_done
  classify_same:
    StrCpy $InstallVersionRelation "${RELATION_SAME}"
    Goto classify_done
  classify_older:
    StrCpy $InstallVersionRelation "${RELATION_OLDER}"
    Goto classify_done
  classify_reject:
    SetErrorLevel 2
    Abort
  classify_done:
FunctionEnd

Function EnforceInstallPolicy
  ${If} $InstallVersionRelation == "${RELATION_OLDER}"
    SetErrorLevel 2
    Abort
  ${EndIf}
  ${If} $ExistingInstallKind == "wix"
    ${If} $SilentMode == 1
    ${OrIf} $PassiveMode == 1
    ${OrIf} $UpdaterMode == 1
      SetErrorLevel 2
      Abort
    ${EndIf}
  ${EndIf}
  ${If} $UpdaterMode == 1
    ${If} $PassiveMode != 1
      SetErrorLevel 2
      Abort
    ${EndIf}
    ${If} $ExistingInstallKind != "nsis"
      SetErrorLevel 2
      Abort
    ${EndIf}
    ${If} $InstallVersionRelation != "${RELATION_NEWER}"
      SetErrorLevel 2
      Abort
    ${EndIf}
  ${EndIf}
FunctionEnd

Function .onInit
  Call EnforceSupportedWindows

  StrCpy $SilentMode 0
  IfSilent 0 +2
    StrCpy $SilentMode 1
  StrCpy $PassiveMode 0
  ${GetOptions} $CMDLINE "/P" $PassiveMode
  IfErrors +2 0
    StrCpy $PassiveMode 1

  StrCpy $UpdaterMode 0
  ${GetOptions} $CMDLINE "/AIRWIKIUPDATE" $UpdaterMode
  IfErrors +2 0
    StrCpy $UpdaterMode 1

  !insertmacro SetContext
  Call ClassifyExistingInstallation
  Call EnforceInstallPolicy

  !if "${DISPLAYLANGUAGESELECTOR}" == "true"
    !insertmacro MUI_LANGDLL_DISPLAY
  !endif

  ${If} $INSTDIR == ""
    ; Set default install location
    !if "${INSTALLMODE}" == "perMachine"
      ${If} ${RunningX64}
        !if "${ARCH}" == "x64"
          StrCpy $INSTDIR "$PROGRAMFILES64\${PRODUCTNAME}"
        !else if "${ARCH}" == "arm64"
          StrCpy $INSTDIR "$PROGRAMFILES64\${PRODUCTNAME}"
        !else
          StrCpy $INSTDIR "$PROGRAMFILES\${PRODUCTNAME}"
        !endif
      ${Else}
        StrCpy $INSTDIR "$PROGRAMFILES\${PRODUCTNAME}"
      ${EndIf}
    !else if "${INSTALLMODE}" == "currentUser"
      StrCpy $INSTDIR "$LOCALAPPDATA\${PRODUCTNAME}"
    !endif

    Call RestorePreviousInstallLocation
  ${EndIf}


  !if "${INSTALLMODE}" == "both"
    !insertmacro MULTIUSER_INIT
  !endif
FunctionEnd


{{#if preinstall_section}}
{{unescape_newlines preinstall_section}}
{{/if}}

!macro CheckIfAppIsRunning
  nsis_tauri_utils::FindProcess "${MAINBINARYNAME}.exe"
  Pop $R0
  ${If} $R0 = 0
      IfSilent kill 0
      ${IfThen} $PassiveMode != 1 ${|} MessageBox MB_OKCANCEL "$(appRunningOkKill)" IDOK kill IDCANCEL cancel ${|}
      kill:
        nsis_tauri_utils::KillProcess "${MAINBINARYNAME}.exe"
        Pop $R0
        Sleep 500
        ${If} $R0 = 0
          Goto app_check_done
        ${Else}
          IfSilent silent ui
          silent:
            System::Call 'kernel32::AttachConsole(i -1)i.r0'
            ${If} $0 != 0
              System::Call 'kernel32::GetStdHandle(i -11)i.r0'
              System::call 'kernel32::SetConsoleTextAttribute(i r0, i 0x0004)' ; set red color
              FileWrite $0 "$(appRunning)$\n"
            ${EndIf}
            Abort
          ui:
            Abort "$(failedToKillApp)"
        ${EndIf}
      cancel:
        Abort "$(appRunning)"
  ${EndIf}
  app_check_done:
!macroend

; The in-app updater launches this installer before asking the eframe process
; to exit cleanly. Give MCP, LAN, watchers and the local model a bounded window
; to stop before the existing recovery path terminates a stuck process.
Function WaitForAirWikiUpdateShutdown
  ${GetOptions} $CMDLINE "/AIRWIKIUPDATE" $R0
  IfErrors update_shutdown_done
  StrCpy $R1 0
  update_shutdown_wait:
    nsis_tauri_utils::FindProcess "${MAINBINARYNAME}.exe"
    Pop $R0
    ${If} $R0 != 0
      Goto update_shutdown_done
    ${EndIf}
    IntOp $R1 $R1 + 1
    ${If} $R1 >= 50
      Goto update_shutdown_done
    ${EndIf}
    Sleep 100
    Goto update_shutdown_wait
  update_shutdown_done:
FunctionEnd

Section Install
  SetOutPath $INSTDIR

  Call WaitForAirWikiUpdateShutdown
  !insertmacro CheckIfAppIsRunning

  ; Copy main executable
  File "${MAINBINARYSRCPATH}"

  ; Create resources directory structure
  {{#each resources_dirs}}
    CreateDirectory "$INSTDIR\\{{this}}"
  {{/each}}

  ; Copy resources
  {{#each resources}}
    File /a "/oname={{this}}" "{{@key}}"
  {{/each}}

  ; Copy external binaries
  {{#each binaries}}
    File /a "/oname={{this}}" "{{@key}}"
  {{/each}}

  ; Create file associations
  {{#each file_associations as |association| ~}}
    {{#each association.extensions as |ext| ~}}
       !insertmacro APP_ASSOCIATE "{{ext}}" "{{or association.name ext}}" "{{association-description association.description ext}}" "$INSTDIR\${MAINBINARYNAME}.exe,0" "Open with ${PRODUCTNAME}" "$INSTDIR\${MAINBINARYNAME}.exe $\"%1$\""
    {{/each}}
  {{/each}}

  ; Register deep links
  {{#each deep_link_protocols as |protocol| ~}}
    WriteRegStr SHCTX "Software\Classes\\{{protocol}}" "URL Protocol" ""
    WriteRegStr SHCTX "Software\Classes\\{{protocol}}" "" "URL:${BUNDLEID} protocol"
    WriteRegStr SHCTX "Software\Classes\\{{protocol}}\DefaultIcon" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\",0"
    WriteRegStr SHCTX "Software\Classes\\{{protocol}}\shell\open\command" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\""
  {{/each}}

  ; Create uninstaller
  WriteUninstaller "$INSTDIR\uninstall.exe"

  ; Save $INSTDIR in registry for future installations
  WriteRegStr SHCTX "${MANUPRODUCTKEY}" "" $INSTDIR

  !if "${INSTALLMODE}" == "both"
    ; Save install mode to be selected by default for the next installation such as updating
    ; or when uninstalling
    WriteRegStr SHCTX "${UNINSTKEY}" $MultiUser.InstallMode 1
  !endif

  ; Registry information for add/remove programs
  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayName" "${PRODUCTNAME}"
  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayIcon" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\""
  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr SHCTX "${UNINSTKEY}" "Publisher" "${MANUFACTURER}"
  WriteRegStr SHCTX "${UNINSTKEY}" "InstallLocation" "$\"$INSTDIR$\""
  WriteRegStr SHCTX "${UNINSTKEY}" "UninstallString" "$\"$INSTDIR\uninstall.exe$\""
  WriteRegDWORD SHCTX "${UNINSTKEY}" "NoModify" "1"
  WriteRegDWORD SHCTX "${UNINSTKEY}" "NoRepair" "1"
  WriteRegDWORD SHCTX "${UNINSTKEY}" "EstimatedSize" "${ESTIMATEDSIZE}"

  ; Create start menu shortcut (GUI)
  !insertmacro MUI_STARTMENU_WRITE_BEGIN Application
    Call CreateStartMenuShortcut
  !insertmacro MUI_STARTMENU_WRITE_END

  ; Create shortcuts for silent and passive installers, which
  ; can be disabled by passing `/NS` flag
  ; GUI installer has buttons for users to control creating them
  IfSilent check_ns_flag 0
  ${IfThen} $PassiveMode == 1 ${|} Goto check_ns_flag ${|}
  Goto shortcuts_done
  check_ns_flag:
    ${GetOptions} $CMDLINE "/NS" $R0
    IfErrors 0 shortcuts_done
      Call CreateDesktopShortcut
      Call CreateStartMenuShortcut
  shortcuts_done:

  ; Auto close this page for passive mode
  ${IfThen} $PassiveMode == 1 ${|} SetAutoClose true ${|}
SectionEnd

Function .onInstSuccess
  ; Check for `/R` flag only in silent and passive installers because
  ; GUI installer has a toggle for the user to (re)start the app
  IfSilent check_r_flag 0
  ${IfThen} $PassiveMode == 1 ${|} Goto check_r_flag ${|}
  Goto run_done
  check_r_flag:
    ${GetOptions} $CMDLINE "/R" $R0
    IfErrors run_done 0
      Exec '"$INSTDIR\${MAINBINARYNAME}.exe"'
  run_done:
FunctionEnd

Function un.onInit
  !insertmacro SetContext

  !if "${INSTALLMODE}" == "both"
    !insertmacro MULTIUSER_UNINIT
  !endif

  !insertmacro MUI_UNGETLANGUAGE
FunctionEnd

Section Uninstall
  !insertmacro CheckIfAppIsRunning

  ; Delete only the exact per-user autostart command managed by AirWiki.
  ; A value with the same name but different bytes is a conflict and is preserved.
  ReadRegStr $R0 HKCU "${AUTOSTARTKEY}" "${AUTOSTARTVALUENAME}"
  StrCpy $R1 "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" --background"
  StrCmp $R0 $R1 0 autostart_cleanup_done
    DeleteRegValue HKCU "${AUTOSTARTKEY}" "${AUTOSTARTVALUENAME}"
  autostart_cleanup_done:

  ; The elevated, same-publisher helper remains the only process allowed to
  ; reconcile firewall rules. Failure or UAC cancellation never blocks uninstall.
  ${If} $RemoveFirewallCheckboxState == ${BST_CHECKED}
    ClearErrors
    ExecShellWait "runas" "$INSTDIR\${FIREWALLHELPER}" "remove" SW_SHOWNORMAL $R0
    ${If} ${Errors}
    ${OrIf} $R0 != 0
      MessageBox MB_OK|MB_ICONEXCLAMATION "$(firewallRulesRemain)"
    ${EndIf}
  ${EndIf}

  ; Delete the app directory and its content from disk
  ; Copy main executable
  Delete "$INSTDIR\${MAINBINARYNAME}.exe"

  ; Delete resources
  {{#each resources}}
    Delete "$INSTDIR\\{{this}}"
  {{/each}}

  ; Delete external binaries
  {{#each binaries}}
    Delete "$INSTDIR\\{{this}}"
  {{/each}}

  ; Delete app associations
  {{#each file_associations as |association| ~}}
    {{#each association.ext as |ext| ~}}
      !insertmacro APP_UNASSOCIATE "{{ext}}" "{{or association.name ext}}"
    {{/each}}
  {{/each}}

  ; Delete deep links
  {{#each deep_link_protocols as |protocol| ~}}
    ReadRegStr $R7 SHCTX "Software\Classes\\{{protocol}}\shell\open\command" ""
    !if $R7 == "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\""
      DeleteRegKey SHCTX "Software\Classes\\{{protocol}}"
    !endif
  {{/each}}

  ; Delete uninstaller
  Delete "$INSTDIR\uninstall.exe"

  {{#each resources_dirs}}
  RMDir /REBOOTOK "$INSTDIR\\{{this}}"
  {{/each}}
  ; A resource targeted at integrations/bridge makes cargo-packager emit only
  ; the leaf directory. Remove the empty app-owned parent as well.
  RMDir "$INSTDIR\integrations"
  RMDir "$INSTDIR"

  ; Remove start menu shortcut
  !insertmacro MUI_STARTMENU_GETFOLDER Application $AppStartMenuFolder
  Delete "$SMPROGRAMS\$AppStartMenuFolder\${PRODUCTNAME}.lnk"
  RMDir "$SMPROGRAMS\$AppStartMenuFolder"

  ; Remove desktop shortcuts
  Delete "$DESKTOP\${PRODUCTNAME}.lnk"

  ; Remove registry information for add/remove programs
  ReadRegStr $R0 SHCTX "${UNINSTKEY}" "UninstallString"
  StrCpy $R1 "$\"$INSTDIR\uninstall.exe$\""
  StrCmp $R0 $R1 0 uninstall_registry_cleanup_done
    DeleteRegKey SHCTX "${UNINSTKEY}"
  uninstall_registry_cleanup_done:

  ; Preserve a product key changed by another installation or administrator.
  ReadRegStr $R0 SHCTX "${MANUPRODUCTKEY}" ""
  StrCmp $R0 $INSTDIR 0 product_registry_cleanup_done
    DeleteRegValue HKCU "${MANUPRODUCTKEY}" "Installer Language"
    DeleteRegValue SHCTX "${MANUPRODUCTKEY}" ""
    DeleteRegKey /ifempty SHCTX "${MANUPRODUCTKEY}"
  product_registry_cleanup_done:

  ; Delete app data
  {{#if appdata_paths}}
  ${If} $DeleteAppDataCheckboxState == 1
      SetShellVarContext current
      {{#each appdata_paths}}
      RmDir /r "{{unescape_dollar_sign this}}"
      {{/each}}
  ${EndIf}
  {{/if}}

  ${GetOptions} $CMDLINE "/P" $R0
  IfErrors +2 0
    SetAutoClose true
SectionEnd

Function RestorePreviousInstallLocation
  ReadRegStr $4 SHCTX "${MANUPRODUCTKEY}" ""
  StrCmp $4 "" +2 0
    StrCpy $INSTDIR $4
FunctionEnd

Function SkipIfPassive
  ${IfThen} $PassiveMode == 1  ${|} Abort ${|}
FunctionEnd

Function CreateDesktopShortcut
  CreateShortcut "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
FunctionEnd

Function CreateStartMenuShortcut
  CreateDirectory "$SMPROGRAMS\$AppStartMenuFolder"
  CreateShortcut "$SMPROGRAMS\$AppStartMenuFolder\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
FunctionEnd
