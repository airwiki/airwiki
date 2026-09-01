$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ($env:AIRWIKI_ENABLE_LEGACY_ARTIFACT_SIGNING -cne "true") {
    throw "the retired Artifact Signing/NSIS experiment is disabled; use windows-signpath.yml"
}

$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$ReleaseDir = Join-Path $Root "target\x86_64-pc-windows-msvc\release"
$Bridge = Join-Path $ReleaseDir "airwiki-mcp-bridge.exe"
$Desktop = Join-Path $ReleaseDir "airwiki.exe"
$Helper = Join-Path $ReleaseDir "airwiki-windows-firewall-helper.exe"
$Mcpb = Join-Path $Root "target\mcpb\x86_64-pc-windows-msvc\airwiki-claude.mcpb"
$OutDir = Join-Path $Root "target\packages\windows"
$TauriInstallerDir = Join-Path $ReleaseDir "bundle\nsis"
$SignedConfig = Join-Path $Root "packaging\windows\tauri.release.generated.conf.json"
$SigningTemp = Join-Path $Root "target\packages\windows-signing-temp"
$UninstallerReceiptDir = Join-Path $Root "target\windows-uninstaller"
$UninstallerReceipt = Join-Path $UninstallerReceiptDir "airwiki-uninstall.exe"
$LlamaRuntime = Join-Path $Root "resources\llama\windows-x64"
$LlamaPolicy = Join-Path $Root "packaging\llama-windows-build-policy.json"
$NsisToolCacheRoot = Join-Path $Root "target\.tauri"
$Tauri = Join-Path $Root "apps\desktop\ui\node_modules\.bin\tauri.cmd"
. (Join-Path $PSScriptRoot "windows-signing.ps1")
. (Join-Path $PSScriptRoot "windows-runtime.ps1")
. (Join-Path $PSScriptRoot "windows-safe-staging.ps1")

if ([string]::IsNullOrWhiteSpace($env:TAURI_SIGNING_PRIVATE_KEY)) {
    throw "Tauri updater private key is required"
}
if ([string]::IsNullOrWhiteSpace($env:AIRWIKI_UPDATER_PUBLIC_KEY)) {
    throw "updater public key is required for post-signing verification"
}

$UsePrebuiltMcpb = $env:AIRWIKI_USE_PREBUILT_MCPB -eq "true"
if (-not [string]::IsNullOrWhiteSpace($env:AIRWIKI_USE_PREBUILT_MCPB) -and
    $env:AIRWIKI_USE_PREBUILT_MCPB -ne "true" -and
    $env:AIRWIKI_USE_PREBUILT_MCPB -ne "false") {
    throw "AIRWIKI_USE_PREBUILT_MCPB must be true or false"
}

function Write-SignedTauriConfig([string] $Destination) {
    $TemplatePath = Join-Path $Root "packaging\windows\tauri.bundle.conf.json"
    $Template = [IO.File]::ReadAllText($TemplatePath) | ConvertFrom-Json
    if ($null -ne $Template.bundle.windows.signCommand) {
        throw "tracked Windows Tauri config must not contain a release signing command"
    }
    $SigningScript = (Resolve-Path (Join-Path $PSScriptRoot "sign-windows-artifact.ps1")).Path
    $SignCommand = [ordered] @{
        cmd = "pwsh.exe"
        args = @(
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            $SigningScript,
            "%1"
        )
    }
    $Template.bundle.windows | Add-Member -NotePropertyName signCommand -NotePropertyValue $SignCommand
    $Materialized = $Template | ConvertTo-Json -Depth 20
    if (Test-Path -LiteralPath $Destination) {
        $Existing = [IO.File]::ReadAllText($Destination) | ConvertFrom-Json
        if ($Existing.bundle.windows.signCommand.cmd -ne "pwsh.exe") {
            throw "refusing to overwrite a non-generated release config"
        }
        Remove-AirWikiWindowsStagingPath `
            -Path $Destination `
            -AllowedRoot (Join-Path $Root "packaging\windows") `
            -Label "generated Windows packager configuration"
    }
    [IO.File]::WriteAllText($Destination, "$Materialized`n", [Text.UTF8Encoding]::new($false))
}

$DesktopSigner = Assert-ExpectedWindowsSigner $Desktop
$BridgeSigner = Assert-ExpectedWindowsSigner $Bridge
$HelperSigner = Assert-ExpectedWindowsSigner $Helper
Assert-SameWindowsSigner $DesktopSigner $BridgeSigner "MCP bridge"
Assert-SameWindowsSigner $DesktopSigner $HelperSigner "firewall helper"
Assert-WindowsFirewallHelperManifest $Helper "signed Windows firewall helper"
$null = Get-WindowsLlamaRuntimeManifest $LlamaRuntime $LlamaPolicy
Assert-WindowsDesktopEmbedsLlamaRuntimeHash $Desktop $LlamaRuntime $LlamaPolicy
$Tauri = Get-VerifiedWindowsRegularFile $Tauri "pinned Tauri CLI"
$TauriVersion = (& $Tauri --version 2>&1 | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $TauriVersion -ne "tauri-cli 2.11.4") {
    throw "Tauri CLI 2.11.4 is required"
}
& (Join-Path $PSScriptRoot "prepare-verified-nsis-toolchain.ps1") `
    -ToolCacheRoot $NsisToolCacheRoot | Out-Null

Push-Location $Root
$PreviousTemp = $env:TEMP
$PreviousTmp = $env:TMP
try {
    & cargo run --locked -p xtask -- packaging verify-updater-embedded-key `
        --binary $Desktop
    if ($LASTEXITCODE -ne 0) {
        throw "desktop binary does not contain the configured updater public key"
    }
    Remove-AirWikiWindowsStagingPath `
        -Path $OutDir `
        -AllowedRoot (Join-Path $Root "target") `
        -Label "signed Windows package output"
    Remove-AirWikiWindowsStagingPath `
        -Path $TauriInstallerDir `
        -AllowedRoot (Join-Path $Root "target") `
        -Label "signed Tauri NSIS output"
    Remove-AirWikiWindowsStagingPath `
        -Path $UninstallerReceiptDir `
        -AllowedRoot (Join-Path $Root "target") `
        -Label "Windows uninstaller receipt staging"

    if ($UsePrebuiltMcpb) {
        $ResolvedMcpb = Get-VerifiedWindowsRegularFile $Mcpb "prebuilt Claude MCPB"
        if (-not $ResolvedMcpb.Equals(
            [IO.Path]::GetFullPath($Mcpb),
            [StringComparison]::OrdinalIgnoreCase
        )) {
            throw "prebuilt Claude MCPB resolved outside its fixed artifact path"
        }
    } else {
        & cargo run --locked -p xtask -- packaging verify-windows-uninstaller
        if ($LASTEXITCODE -ne 0) { throw "Windows uninstaller policy validation failed" }

        & cargo run --locked -p xtask -- mcpb build `
            --target x86_64-pc-windows-msvc `
            --bridge $Bridge `
            --output $Mcpb
        if ($LASTEXITCODE -ne 0) { throw "Claude MCPB build failed" }
    }

    Write-SignedTauriConfig $SignedConfig
    Remove-AirWikiWindowsStagingPath `
        -Path $SigningTemp `
        -AllowedRoot (Join-Path $Root "target") `
        -Label "Windows signing staging"
    New-Item -ItemType Directory -Path $SigningTemp -Force | Out-Null
    # NSIS passes !uninstfinalize a generated .tmp PE. Keeping TEMP inside
    # target lets the closed signing wrapper reject arbitrary filesystem paths.
    $env:TEMP = $SigningTemp
    $env:TMP = $SigningTemp
    Push-Location (Join-Path $Root "apps\desktop")
    try {
        & $Tauri bundle `
            --ci `
            --config $SignedConfig `
            --target x86_64-pc-windows-msvc `
            --bundles nsis
        if ($LASTEXITCODE -ne 0) { throw "Tauri signed NSIS packaging failed" }
    } finally {
        Pop-Location
    }
    $Installers = @(Get-ChildItem -LiteralPath $TauriInstallerDir -File -Filter *.exe)
    if ($Installers.Count -ne 1) { throw "Expected exactly one signed Tauri NSIS installer" }
    New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
    $FinalInstaller = Join-Path $OutDir $Installers[0].Name
    Copy-Item -LiteralPath $Installers[0].FullName -Destination $FinalInstaller
    $UninstallerSigner = Assert-ExpectedWindowsSigner $UninstallerReceipt
    Assert-SameWindowsSigner $DesktopSigner $UninstallerSigner "generated uninstaller"

    & $Tauri signer sign $FinalInstaller
    if ($LASTEXITCODE -ne 0) { throw "Tauri updater signing failed" }
    $UpdaterSignature = Get-VerifiedWindowsRegularFile `
        "$FinalInstaller.sig" `
        "Tauri updater signature"
    & cargo run --locked -p xtask -- packaging verify-updater-signature `
        --artifact $FinalInstaller `
        --signature $UpdaterSignature
    if ($LASTEXITCODE -ne 0) { throw "Tauri updater signature verification failed" }
} finally {
    $env:TEMP = $PreviousTemp
    $env:TMP = $PreviousTmp
    Remove-AirWikiWindowsStagingPath `
        -Path $SigningTemp `
        -AllowedRoot (Join-Path $Root "target") `
        -Label "Windows signing staging"
    Remove-AirWikiWindowsStagingPath `
        -Path $SignedConfig `
        -AllowedRoot (Join-Path $Root "packaging\windows") `
        -Label "generated Windows packager configuration"
    Pop-Location
}
