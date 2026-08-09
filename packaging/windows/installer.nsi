; Based on Tauri bundler 2.9.4's NSIS template, with AirWiki's fixed-path,
; update, autostart, payload and opt-in cleanup policy kept in this file.
Unicode true
ManifestDPIAware true
ManifestDPIAwareness PerMonitorV2

!if "{{compression}}" == "none"
  SetCompress off
!else
  SetCompressor /SOLID "{{compression}}"
!endif

{{#if signed_plugins_path}}
!addplugindir "{{signed_plugins_path}}"
{{/if}}

!include MUI2.nsh
!include FileFunc.nsh
!include x64.nsh
!include WinVer.nsh
!include WordFunc.nsh
!include "utils.nsh"
!include "FileAssociation.nsh"
!include "Win\COM.nsh"
!include "Win\Propkey.nsh"
!include "StrFunc.nsh"
${StrCase}
${StrLoc}

{{#if installer_hooks}}
!include "{{installer_hooks}}"
{{/if}}

!define WEBVIEW2APPGUID "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"
!define MANUFACTURER "{{manufacturer}}"
!define PRODUCTNAME "{{product_name}}"
!define VERSION "{{version}}"
!define VERSIONWITHBUILD "{{version_with_build}}"
!define HOMEPAGE "{{homepage}}"
!define INSTALLMODE "{{install_mode}}"
!define LICENSE "{{license}}"
!define INSTALLERICON "{{installer_icon}}"
!define SIDEBARIMAGE "{{sidebar_image}}"
!define HEADERIMAGE "{{header_image}}"
!define UNINSTALLERICON "{{uninstaller_icon}}"
!define UNINSTALLERHEADERIMAGE "{{uninstaller_header_image}}"
!define MAINBINARYNAME "{{main_binary_name}}"
!define MAINBINARYSRCPATH "{{main_binary_path}}"
!define BUNDLEID "{{bundle_id}}"
!define COPYRIGHT "{{copyright}}"
!define OUTFILE "{{out_file}}"
!define ARCH "{{arch}}"
!define ADDITIONALPLUGINSPATH "{{additional_plugins_path}}"
!define ALLOWDOWNGRADES "{{allow_downgrades}}"
!define DISPLAYLANGUAGESELECTOR "{{display_language_selector}}"
!define INSTALLWEBVIEW2MODE "{{install_webview2_mode}}"
!define WEBVIEW2INSTALLERARGS "{{webview2_installer_args}}"
!define WEBVIEW2BOOTSTRAPPERPATH "{{webview2_bootstrapper_path}}"
!define WEBVIEW2INSTALLERPATH "{{webview2_installer_path}}"
!define MINIMUMWEBVIEW2VERSION "{{minimum_webview2_version}}"
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
!define PAYLOAD_REMOVAL_ATTEMPTS 150
!define PAYLOAD_REMOVAL_DELAY_MS 100
!define UNINSTROOT "Software\Microsoft\Windows\CurrentVersion\Uninstall"

; Declare state before page callbacks are compiled. LogicLib otherwise treats
; forward variable references as constants and silently weakens the checks.
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
Var ExistingNsisInstallLocation
Var ManagedPayloadPath
Var ManagedPayloadAttempts
Var ManagedValidationPath
Var ManagedValidationParent

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
VIAddVersionKey "FileDescription" "${PRODUCTNAME}"
VIAddVersionKey "LegalCopyright" "${COPYRIGHT}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"

!addplugindir "${ADDITIONALPLUGINSPATH}"

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

; 5. Start menu shortcut page. The current-user binary directory is fixed so
; path aliases cannot overlap the local-first data roots.
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
Var AppStartMenuFolder
!insertmacro MUI_PAGE_STARTMENU Application $AppStartMenuFolder

; 6. Installation page
!insertmacro MUI_PAGE_INSTFILES

; 7. Finish page
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
Var DeleteAppDataCheckbox
Var DeleteAppDataCheckboxState
!define /ifndef WS_EX_LAYOUTRTL         0x00400000
!define MUI_PAGE_CUSTOMFUNCTION_SHOW un.ConfirmShow
Function un.ConfirmShow
    FindWindow $1 "#32770" "" $HWNDPARENT ; Find inner dialog
    ${If} $(^RTL) == 1
      System::Call 'USER32::CreateWindowEx(i${__NSD_CheckBox_EXSTYLE}|${WS_EX_LAYOUTRTL},t"${__NSD_CheckBox_CLASS}",t "$(removeFirewallRules)",i${__NSD_CheckBox_STYLE},i 50,i 75,i 400, i 25,i$1,i0,i0,i0)i.s'
      System::Call 'USER32::CreateWindowEx(i${__NSD_CheckBox_EXSTYLE}|${WS_EX_LAYOUTRTL},t"${__NSD_CheckBox_CLASS}",t "$(deleteAppData)",i${__NSD_CheckBox_STYLE},i 50,i 100,i 400, i 25,i$1,i0,i0,i0)i.s'
    ${Else}
      System::Call 'USER32::CreateWindowEx(i${__NSD_CheckBox_EXSTYLE},t"${__NSD_CheckBox_CLASS}",t "$(removeFirewallRules)",i${__NSD_CheckBox_STYLE},i 0,i 75,i 400, i 25,i$1,i0,i0,i0)i.s'
      System::Call 'USER32::CreateWindowEx(i${__NSD_CheckBox_EXSTYLE},t"${__NSD_CheckBox_CLASS}",t "$(deleteAppData)",i${__NSD_CheckBox_STYLE},i 0,i 100,i 400, i 25,i$1,i0,i0,i0)i.s'
    ${EndIf}
    Pop $DeleteAppDataCheckbox
    Pop $RemoveFirewallCheckbox
    SendMessage $HWNDPARENT ${WM_GETFONT} 0 0 $1
    SendMessage $RemoveFirewallCheckbox ${WM_SETFONT} $1 1
    SendMessage $DeleteAppDataCheckbox ${WM_SETFONT} $1 1
FunctionEnd
!define MUI_PAGE_CUSTOMFUNCTION_LEAVE un.ConfirmLeave
Function un.ConfirmLeave
    SendMessage $RemoveFirewallCheckbox ${BM_GETCHECK} 0 0 $RemoveFirewallCheckboxState
    SendMessage $DeleteAppDataCheckbox ${BM_GETCHECK} 0 0 $DeleteAppDataCheckboxState
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
LangString UnsafeInstallLocation ${LANG_ENGLISH} "AirWiki binaries must be installed outside AirWiki's local data folders. Uninstall an older development candidate while preserving its data, then install this candidate again."
LangString UnsafeInstallLocation ${LANG_SPANISH} "Los binarios de AirWiki deben instalarse fuera de las carpetas de datos locales. Desinstala un candidato de desarrollo anterior conservando sus datos y vuelve a instalar este candidato."
!insertmacro MUI_RESERVEFILE_LANGDLL
{{#each language_files}}
  !include "{{this}}"
{{/each}}
LangString removeFirewallRules ${LANG_ENGLISH} "Remove AirWiki's restricted local-network firewall rules (administrator approval required)"
LangString removeFirewallRules ${LANG_SPANISH} "Quitar las reglas restringidas de red local de AirWiki (requiere aprobación de administrador)"
LangString firewallRulesRemain ${LANG_ENGLISH} "Windows could not remove the firewall rules. Uninstallation will continue and the rules will remain until removed from Windows Security."
LangString firewallRulesRemain ${LANG_SPANISH} "Windows no pudo quitar las reglas del firewall. La desinstalación continuará y las reglas permanecerán hasta quitarlas desde Seguridad de Windows."
LangString installedPayloadRemovalFailed ${LANG_ENGLISH} "Windows could not remove AirWiki's installed application files. Uninstallation stopped without deleting local data."
LangString installedPayloadRemovalFailed ${LANG_SPANISH} "Windows no pudo quitar los archivos instalados de AirWiki. La desinstalación se detuvo sin borrar los datos locales."
LangString untrustedUninstallState ${LANG_ENGLISH} "AirWiki could not verify this uninstaller against the fixed per-user installation. No application files or local data were removed."
LangString untrustedUninstallState ${LANG_SPANISH} "AirWiki no pudo verificar este desinstalador contra la instalación fija del usuario. No se quitaron archivos de la aplicación ni datos locales."

!macro AirWikiSetContext
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
  StrCpy $ExistingNsisInstallLocation ""

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
    ReadRegStr $7 SHCTX "${MANUPRODUCTKEY}" ""
    ${If} $NsisMetadataState == "${NSIS_METADATA_ABSENT}"
      ${If} $7 != ""
        Goto classify_reject
      ${EndIf}
    ${ElseIf} $NsisMetadataState == "${NSIS_METADATA_COMPLETE}"
      ${If} $7 == ""
        Goto classify_reject
      ${EndIf}
      StrCpy $8 "$\"$7$\""
      StrCmp $4 $8 0 classify_reject
      StrCpy $8 "$\"$7\uninstall.exe$\""
      StrCmp $5 $8 0 classify_reject
      StrCpy $ExistingNsisInstallLocation $7
    ${EndIf}
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

Function RejectUnsafeInstallLocation
  IfSilent unsafe_install_location_abort
  MessageBox MB_OK|MB_ICONSTOP "$(UnsafeInstallLocation)"
  unsafe_install_location_abort:
    SetErrorLevel 2
    Abort
FunctionEnd

Function ValidateInstallLocation
  ClearErrors
  GetFullPathName $0 "$LOCALAPPDATA"
  IfErrors unsafe_install_location
  StrCpy $0 "$0\Programs\${PRODUCTNAME}"
  StrCpy $1 "$INSTDIR"
  ${StrCase} $0 $0 "L"
  ${StrCase} $1 $1 "L"
  StrCmp $0 $1 install_location_valid unsafe_install_location

  install_location_valid:
  System::Call 'kernel32::GetFileAttributesW(w "$LOCALAPPDATA\Programs")i .r2'
  StrCmp $2 -1 install_location_leaf_attributes
  IntOp $3 $2 & 0x0400
  StrCmp $3 0 install_location_leaf_attributes unsafe_install_location
  install_location_leaf_attributes:
  System::Call 'kernel32::GetFileAttributesW(w "$LOCALAPPDATA\Programs\${PRODUCTNAME}")i .r2'
  StrCmp $2 -1 install_location_safe
  IntOp $3 $2 & 0x0400
  StrCmp $3 0 install_location_safe unsafe_install_location
  install_location_safe:
  Return

  unsafe_install_location:
    Call RejectUnsafeInstallLocation
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

!insertmacro AirWikiSetContext
  Call ClassifyExistingInstallation
  Call EnforceInstallPolicy

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
      ; Keep installed binaries outside the local-first data root
      ; ($LOCALAPPDATA\airwiki\AirWiki on case-insensitive Windows).
      StrCpy $INSTDIR "$LOCALAPPDATA\Programs\${PRODUCTNAME}"
    !endif

    Call RestorePreviousInstallLocation
  ${EndIf}

  Call ValidateInstallLocation

  !if "${DISPLAYLANGUAGESELECTOR}" == "true"
    !insertmacro MUI_LANGDLL_DISPLAY
  !endif

  !if "${INSTALLMODE}" == "both"
    !insertmacro MULTIUSER_INIT
  !endif
FunctionEnd


; The in-app updater launches this installer before asking the Tauri process
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

Section WebView2
  ${If} ${RunningX64}
    ReadRegStr $4 HKLM "SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\${WEBVIEW2APPGUID}" "pv"
  ${Else}
    ReadRegStr $4 HKLM "SOFTWARE\Microsoft\EdgeUpdate\Clients\${WEBVIEW2APPGUID}" "pv"
  ${EndIf}
  ${If} $4 == ""
    ReadRegStr $4 HKCU "SOFTWARE\Microsoft\EdgeUpdate\Clients\${WEBVIEW2APPGUID}" "pv"
  ${EndIf}

  ${If} $4 == ""
    ; An update never needs to bootstrap WebView2: the installed Tauri app
    ; already proved that the runtime exists. A fresh install communicates a
    ; network failure and remains on this page so the user can retry.
    ${If} $UpdaterMode != 1
      !if "${INSTALLWEBVIEW2MODE}" == "downloadBootstrapper"
        Delete "$TEMP\MicrosoftEdgeWebview2Setup.exe"
        DetailPrint "$(webview2Downloading)"
        NSISdl::download "https://go.microsoft.com/fwlink/p/?LinkId=2124703" "$TEMP\MicrosoftEdgeWebview2Setup.exe"
        Pop $0
        ${If} $0 == "success"
          DetailPrint "$(webview2DownloadSuccess)"
        ${Else}
          DetailPrint "$(webview2DownloadError)"
          Abort "$(webview2AbortError)"
        ${EndIf}
        StrCpy $6 "$TEMP\MicrosoftEdgeWebview2Setup.exe"
        Goto install_webview2
      !endif

      !if "${INSTALLWEBVIEW2MODE}" == "embedBootstrapper"
        Delete "$TEMP\MicrosoftEdgeWebview2Setup.exe"
        File "/oname=$TEMP\MicrosoftEdgeWebview2Setup.exe" "${WEBVIEW2BOOTSTRAPPERPATH}"
        StrCpy $6 "$TEMP\MicrosoftEdgeWebview2Setup.exe"
        Goto install_webview2
      !endif

      !if "${INSTALLWEBVIEW2MODE}" == "offlineInstaller"
        Delete "$TEMP\MicrosoftEdgeWebView2RuntimeInstaller.exe"
        File "/oname=$TEMP\MicrosoftEdgeWebView2RuntimeInstaller.exe" "${WEBVIEW2INSTALLERPATH}"
        StrCpy $6 "$TEMP\MicrosoftEdgeWebView2RuntimeInstaller.exe"
        Goto install_webview2
      !endif

      Goto webview2_done

      install_webview2:
        DetailPrint "$(installingWebview2)"
        ExecWait "$6 ${WEBVIEW2INSTALLERARGS} /install" $1
        ${If} $1 = 0
          DetailPrint "$(webview2InstallSuccess)"
        ${Else}
          DetailPrint "$(webview2InstallError)"
          Abort "$(webview2AbortError)"
        ${EndIf}
      webview2_done:
    ${EndIf}
  ${Else}
    !if "${MINIMUMWEBVIEW2VERSION}" != ""
      ${VersionCompare} "${MINIMUMWEBVIEW2VERSION}" "$4" $R0
      ${If} $R0 = 1
        update_webview:
          DetailPrint "$(installingWebview2)"
          ${If} ${RunningX64}
            ReadRegStr $R1 HKLM "SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate" "path"
          ${Else}
            ReadRegStr $R1 HKLM "SOFTWARE\Microsoft\EdgeUpdate" "path"
          ${EndIf}
          ${If} $R1 == ""
            ReadRegStr $R1 HKCU "SOFTWARE\Microsoft\EdgeUpdate" "path"
          ${EndIf}
          ${If} $R1 != ""
            ExecWait `"$R1" /install appguid=${WEBVIEW2APPGUID}&needsadmin=true` $1
            ${If} $1 = 0
              DetailPrint "$(webview2InstallSuccess)"
            ${Else}
              MessageBox MB_ICONEXCLAMATION|MB_ABORTRETRYIGNORE "$(webview2InstallError)" IDIGNORE ignore IDRETRY update_webview
              Quit
              ignore:
            ${EndIf}
          ${EndIf}
      ${EndIf}
    !endif
  ${EndIf}
SectionEnd

Section Install
  ; Revalidate the effective command-line/default path before every write.
  Call ValidateInstallLocation
  SetOutPath $INSTDIR

  Call WaitForAirWikiUpdateShutdown
  !insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"

  ; Copy main executable
  File "${MAINBINARYSRCPATH}"

  ; Create resources directory structure
  {{#each resources_dirs}}
    CreateDirectory "$INSTDIR\\{{this}}"
  {{/each}}

  ; Copy resources
  {{#each resources}}
    File /a "/oname={{this.[1]}}" "{{no-escape @key}}"
  {{/each}}

  ; Copy external binaries
  {{#each binaries}}
    File /a "/oname={{this}}" "{{no-escape @key}}"
  {{/each}}

  ; Create file associations
  {{#each file_associations as |association| ~}}
    {{#each association.ext as |ext| ~}}
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

Function un.RejectUntrustedUninstallState
  SetErrorLevel 2
  Abort "$(untrustedUninstallState)"
FunctionEnd

Function un.ValidateUninstallAuthority
  ClearErrors
  GetFullPathName $0 "$LOCALAPPDATA"
  IfErrors untrusted_uninstall_state
  StrCpy $0 "$0\Programs\${PRODUCTNAME}"
  GetFullPathName $1 "$INSTDIR"
  IfErrors untrusted_uninstall_state
  System::Call 'kernel32::lstrcmpiW(w r0, w r1)i .r2'
  StrCmp $2 0 uninstall_path_matches untrusted_uninstall_state

  uninstall_path_matches:
  System::Call 'kernel32::GetFileAttributesW(w "$LOCALAPPDATA\Programs")i .r2'
  StrCmp $2 -1 untrusted_uninstall_state
  IntOp $3 $2 & 0x0010
  StrCmp $3 0 untrusted_uninstall_state
  IntOp $3 $2 & 0x0400
  StrCmp $3 0 uninstall_leaf_attributes untrusted_uninstall_state

  uninstall_leaf_attributes:
  System::Call 'kernel32::GetFileAttributesW(w "$INSTDIR")i .r2'
  StrCmp $2 -1 untrusted_uninstall_state
  IntOp $3 $2 & 0x0010
  StrCmp $3 0 untrusted_uninstall_state
  IntOp $3 $2 & 0x0400
  StrCmp $3 0 uninstall_binary_attributes untrusted_uninstall_state

  uninstall_binary_attributes:
  System::Call 'kernel32::GetFileAttributesW(w "$INSTDIR\uninstall.exe")i .r2'
  StrCmp $2 -1 untrusted_uninstall_state
  IntOp $3 $2 & 0x0010
  StrCmp $3 0 uninstall_binary_reparse untrusted_uninstall_state
  uninstall_binary_reparse:
  IntOp $3 $2 & 0x0400
  StrCmp $3 0 uninstall_registry_authority untrusted_uninstall_state

  uninstall_registry_authority:
  ReadRegStr $0 SHCTX "${UNINSTKEY}" "DisplayName"
  StrCmp $0 "${PRODUCTNAME}" 0 untrusted_uninstall_state
  ReadRegStr $0 SHCTX "${UNINSTKEY}" "Publisher"
  StrCmp $0 "${MANUFACTURER}" 0 untrusted_uninstall_state
  ReadRegStr $0 SHCTX "${UNINSTKEY}" "DisplayVersion"
  StrCmp $0 "${VERSION}" 0 untrusted_uninstall_state
  ReadRegStr $0 SHCTX "${UNINSTKEY}" "InstallLocation"
  StrCpy $1 "$\"$INSTDIR$\""
  StrCmp $0 $1 0 untrusted_uninstall_state
  ReadRegStr $0 SHCTX "${UNINSTKEY}" "UninstallString"
  StrCpy $1 "$\"$INSTDIR\uninstall.exe$\""
  StrCmp $0 $1 0 untrusted_uninstall_state
  ReadRegStr $0 SHCTX "${MANUPRODUCTKEY}" ""
  StrCmp $0 $INSTDIR uninstall_authority_valid untrusted_uninstall_state

  untrusted_uninstall_state:
    Call un.RejectUntrustedUninstallState
  uninstall_authority_valid:
FunctionEnd

Function un.ValidateManagedPayloadTree
  Push "$INSTDIR\${MAINBINARYNAME}.exe"
  Call un.ValidateManagedPath
  Push "$INSTDIR\uninstall.exe"
  Call un.ValidateManagedPath
  {{#each resources}}
    Push "$INSTDIR\\{{this.[1]}}"
    Call un.ValidateManagedPath
  {{/each}}
  {{#each binaries}}
    Push "$INSTDIR\\{{this}}"
    Call un.ValidateManagedPath
  {{/each}}
  {{#each resources_dirs}}
    Push "$INSTDIR\\{{this}}"
    Call un.ValidateManagedPath
  {{/each}}
  Push "$INSTDIR\integrations"
  Call un.ValidateManagedPath
FunctionEnd

Function un.onInit
!insertmacro AirWikiSetContext

  !if "${INSTALLMODE}" == "both"
    !insertmacro MULTIUSER_UNINIT
  !endif

  !insertmacro MUI_UNGETLANGUAGE
  Call un.ValidateUninstallAuthority
  Call un.ValidateManagedPayloadTree
FunctionEnd

Function un.RemoveManagedFile
  Pop $ManagedPayloadPath
  StrCpy $ManagedPayloadAttempts 0
  managed_file_remove_retry:
    Push "$ManagedPayloadPath"
    Call un.ValidateManagedPath
    ClearErrors
    Delete "$ManagedPayloadPath"
    IfFileExists "$ManagedPayloadPath" 0 managed_file_remove_done
    IntOp $ManagedPayloadAttempts $ManagedPayloadAttempts + 1
    ${If} $ManagedPayloadAttempts >= ${PAYLOAD_REMOVAL_ATTEMPTS}
      SetErrorLevel 2
      Abort "$(installedPayloadRemovalFailed)"
    ${EndIf}
    Sleep ${PAYLOAD_REMOVAL_DELAY_MS}
    Goto managed_file_remove_retry
  managed_file_remove_done:
FunctionEnd

Function un.RemoveManagedDirectory
  Pop $ManagedPayloadPath
  StrCpy $ManagedPayloadAttempts 0
  managed_directory_remove_retry:
    Push "$ManagedPayloadPath"
    Call un.ValidateManagedPath
    ClearErrors
    RMDir "$ManagedPayloadPath"
    IfFileExists "$ManagedPayloadPath\*.*" 0 managed_directory_remove_done
    IntOp $ManagedPayloadAttempts $ManagedPayloadAttempts + 1
    ${If} $ManagedPayloadAttempts >= ${PAYLOAD_REMOVAL_ATTEMPTS}
      SetErrorLevel 2
      Abort "$(installedPayloadRemovalFailed)"
    ${EndIf}
    Sleep ${PAYLOAD_REMOVAL_DELAY_MS}
    Goto managed_directory_remove_retry
  managed_directory_remove_done:
FunctionEnd

Function un.ValidateManagedPath
  Pop $ManagedValidationPath
  managed_path_validation_loop:
    System::Call 'kernel32::GetFileAttributesW(w "$ManagedValidationPath")i .r2'
    StrCmp $2 -1 managed_path_validation_parent
    IntOp $3 $2 & 0x0400
    StrCmp $3 0 managed_path_validation_parent untrusted_managed_path

  managed_path_validation_parent:
    System::Call 'kernel32::lstrcmpiW(w "$ManagedValidationPath", w "$INSTDIR")i .r2'
    StrCmp $2 0 managed_path_valid
    ${GetParent} "$ManagedValidationPath" $ManagedValidationParent
    System::Call 'kernel32::lstrcmpiW(w "$ManagedValidationParent", w "$ManagedValidationPath")i .r2'
    StrCmp $2 0 untrusted_managed_path
    StrCpy $ManagedValidationPath "$ManagedValidationParent"
    Goto managed_path_validation_loop

  untrusted_managed_path:
    Call un.RejectUntrustedUninstallState
  managed_path_valid:
FunctionEnd

Section Uninstall
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
  Push "$INSTDIR\${MAINBINARYNAME}.exe"
  Call un.RemoveManagedFile

  ; Delete resources
  {{#each resources}}
    Push "$INSTDIR\\{{this.[1]}}"
    Call un.RemoveManagedFile
  {{/each}}

  ; Delete external binaries
  {{#each binaries}}
    Push "$INSTDIR\\{{this}}"
    Call un.RemoveManagedFile
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
  Push "$INSTDIR\uninstall.exe"
  Call un.RemoveManagedFile

  {{#each resources_dirs}}
  Push "$INSTDIR\\{{this}}"
  {{/each}}
  ; Pop the sorted directory list in reverse so children are removed first.
  {{#each resources_dirs}}
  Call un.RemoveManagedDirectory
  {{/each}}
  ; The bridge resource contributes only its leaf directory. Remove the empty
  ; app-owned parent as well.
  Push "$INSTDIR\integrations"
  Call un.RemoveManagedDirectory
  Push "$INSTDIR"
  Call un.RemoveManagedDirectory

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

  ; Delete only AirWiki's two documented mutable roots when explicitly chosen.
  ${If} $DeleteAppDataCheckboxState == 1
    SetShellVarContext current
    RmDir /r "$LOCALAPPDATA\airwiki\AirWiki"
    RmDir /r "$APPDATA\airwiki\AirWiki"
  ${EndIf}

  ${GetOptions} $CMDLINE "/P" $R0
  IfErrors +2 0
    SetAutoClose true
SectionEnd

Function RestorePreviousInstallLocation
  ${If} $ExistingInstallKind == "nsis"
    StrCpy $INSTDIR $ExistingNsisInstallLocation
  ${EndIf}
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
