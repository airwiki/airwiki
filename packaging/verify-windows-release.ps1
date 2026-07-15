[CmdletBinding()]
param(
    [string] $SevenZipToolRoot = $env:AIRWIKI_7ZIP_ROOT
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$ReleaseDir = Join-Path $Root "target\x86_64-pc-windows-msvc\release"
$OutDir = Join-Path $Root "target\packages\windows"
$ExpectedVersion = "0.2.0"
$ExpectedWindowsVersion = "$ExpectedVersion.0"
$Bridge = Join-Path $ReleaseDir "airwiki-mcp-bridge.exe"
$Desktop = Join-Path $ReleaseDir "airwiki.exe"
$Helper = Join-Path $ReleaseDir "airwiki-windows-firewall-helper.exe"
$Mcpb = Join-Path $Root "target\mcpb\x86_64-pc-windows-msvc\airwiki-claude.mcpb"
$UninstallerReceipt = Join-Path $Root "target\windows-uninstaller\airwiki-uninstall.exe"
$Installer = Join-Path $OutDir "airwiki_${ExpectedVersion}_x64-setup.exe"
$LlamaRuntime = Join-Path $Root "resources\llama\windows-x64"
$LlamaPolicy = Join-Path $Root "packaging\llama-windows-build-policy.json"
. (Join-Path $PSScriptRoot "windows-signing.ps1")
. (Join-Path $PSScriptRoot "windows-runtime.ps1")
. (Join-Path $PSScriptRoot "windows-payload.ps1")
. (Join-Path $PSScriptRoot "windows-safe-staging.ps1")

if ([string]::IsNullOrWhiteSpace($SevenZipToolRoot)) {
    throw "AIRWIKI_7ZIP_ROOT or -SevenZipToolRoot is required"
}
$ResolvedSevenZipToolRoot = [IO.Path]::GetFullPath($SevenZipToolRoot)
& (Join-Path $PSScriptRoot "prepare-verified-7zip.ps1") `
    -ToolRoot $ResolvedSevenZipToolRoot `
    -ValidateOnly | Out-Null
$SevenZip = Get-VerifiedWindowsRegularFile `
    (Join-Path $ResolvedSevenZipToolRoot "7z.exe") `
    "pinned 7-Zip extractor"

$IsCi = $env:CI -eq "true" -or $env:GITHUB_ACTIONS -eq "true"
$ReleaseXtask = $null
if (-not [string]::IsNullOrWhiteSpace($env:AIRWIKI_XTASK)) {
    $ReleaseXtask = Get-VerifiedWindowsRegularFile $env:AIRWIKI_XTASK "prebuilt xtask"
    if ([IO.Path]::GetFileName($ReleaseXtask) -ne "xtask.exe") {
        throw "prebuilt xtask must be named xtask.exe"
    }
} elseif ($IsCi) {
    throw "AIRWIKI_XTASK is required in CI"
}

function Invoke-ReleaseXtask([string[]] $Arguments, [string] $FailureMessage) {
    if ($null -ne $script:ReleaseXtask) {
        & $script:ReleaseXtask @Arguments
    } else {
        & cargo run --locked -p xtask -- @Arguments
    }
    if ($LASTEXITCODE -ne 0) {
        throw $FailureMessage
    }
}

function Get-SinglePayload([string] $PayloadRoot, [string] $Name) {
    $Matches = @(Get-ChildItem -LiteralPath $PayloadRoot -Recurse -File -Filter $Name)
    if ($Matches.Count -ne 1) { throw "Expected exactly one $Name in the NSIS payload" }
    return $Matches[0].FullName
}

function Assert-SameBytes([string] $Expected, [string] $Actual, [string] $Label) {
    if ((Get-FileHash -LiteralPath $Expected -Algorithm SHA256).Hash -ne
        (Get-FileHash -LiteralPath $Actual -Algorithm SHA256).Hash) {
        throw "$Label in the NSIS payload differs from the signed build artifact"
    }
}

function Assert-ExactWindowsVersion([string] $Path, [string] $Label) {
    $Info = (Get-Item -LiteralPath $Path).VersionInfo
    $FileVersion = "{0}.{1}.{2}.{3}" -f `
        $Info.FileMajorPart, $Info.FileMinorPart, $Info.FileBuildPart, $Info.FilePrivatePart
    $ProductVersion = "{0}.{1}.{2}.{3}" -f `
        $Info.ProductMajorPart, $Info.ProductMinorPart, $Info.ProductBuildPart, $Info.ProductPrivatePart
    if ($FileVersion -ne $script:ExpectedWindowsVersion -or
        $ProductVersion -ne $script:ExpectedWindowsVersion -or
        $Info.FileVersion -ne $script:ExpectedVersion -or
        $Info.ProductVersion -ne $script:ExpectedVersion) {
        throw "$Label version metadata does not exactly match $script:ExpectedVersion"
    }
}

$DesktopSigner = Assert-ExpectedWindowsSigner $Desktop
$BridgeSigner = Assert-ExpectedWindowsSigner $Bridge
$HelperSigner = Assert-ExpectedWindowsSigner $Helper
Assert-SameWindowsSigner $DesktopSigner $BridgeSigner "MCP bridge"
Assert-SameWindowsSigner $DesktopSigner $HelperSigner "firewall helper"
Assert-ExactWindowsVersion $Desktop "desktop"
Assert-ExactWindowsVersion $Helper "firewall helper"
$null = Get-WindowsLlamaRuntimeManifest $LlamaRuntime $LlamaPolicy
Assert-WindowsDesktopEmbedsLlamaRuntimeHash $Desktop $LlamaRuntime $LlamaPolicy
Assert-WindowsFirewallHelperManifest $Helper "signed Windows firewall helper"

$Installers = @(Get-ChildItem -LiteralPath $OutDir -File -Filter *.exe)
if ($Installers.Count -ne 1 -or
    -not $Installers[0].FullName.Equals(
        [IO.Path]::GetFullPath($Installer),
        [StringComparison]::OrdinalIgnoreCase
    )) {
    throw "Expected only the exact AirWiki NSIS installer"
}
$InstallerSigner = Assert-ExpectedWindowsSigner $Installer
Assert-SameWindowsSigner $DesktopSigner $InstallerSigner "NSIS installer"
Assert-ExactWindowsVersion $Installer "NSIS installer"
Assert-WindowsPeMachine $Installer 0x014c "final NSIS installer"
$UninstallerSigner = Assert-ExpectedWindowsSigner $UninstallerReceipt
Assert-SameWindowsSigner $DesktopSigner $UninstallerSigner "generated uninstaller"
Assert-ExactWindowsVersion $UninstallerReceipt "generated uninstaller"
Assert-WindowsPeMachine $UninstallerReceipt 0x014c "generated NSIS uninstaller"

$ExtractDir = Join-Path $Root "target\packages\windows-signed-payload-check"
Remove-AirWikiWindowsStagingPath `
    -Path $ExtractDir `
    -AllowedRoot (Join-Path $Root "target") `
    -Label "signed Windows payload verification staging"
New-Item -ItemType Directory -Path $ExtractDir -Force | Out-Null
try {
    & $SevenZip x -y "-o$ExtractDir" $Installer | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "7-Zip could not inspect the signed NSIS payload" }

    $PackagedDesktop = Get-SinglePayload $ExtractDir "airwiki.exe"
    $PayloadRoot = [IO.Path]::GetDirectoryName($PackagedDesktop)
    $PackagedBridge = Get-VerifiedWindowsRegularFile `
        (Join-Path $PayloadRoot "integrations\bridge\airwiki-mcp-bridge.exe") `
        "packaged MCP bridge"
    $PackagedHelper = Get-VerifiedWindowsRegularFile `
        (Join-Path $PayloadRoot "airwiki-windows-firewall-helper.exe") `
        "packaged firewall helper"
    $PackagedMcpb = Get-VerifiedWindowsRegularFile `
        (Join-Path $PayloadRoot "integrations\airwiki-claude.mcpb") `
        "packaged Claude MCPB"
    $PackagedLicense = Get-VerifiedWindowsRegularFile `
        (Join-Path $PayloadRoot "LICENSE") `
        "packaged project license"
    $PackagedNotices = Get-VerifiedWindowsRegularFile `
        (Join-Path $PayloadRoot "THIRD_PARTY_NOTICES.md") `
        "packaged third-party notices"
    $PackagedLlamaServer = Get-SinglePayload $ExtractDir "llama-server.exe"

    Assert-SameBytes $Desktop $PackagedDesktop "Desktop"
    Assert-SameBytes $Bridge $PackagedBridge "MCP bridge"
    Assert-SameBytes $Helper $PackagedHelper "Firewall helper"
    Assert-SameBytes $Mcpb $PackagedMcpb "Claude MCPB"
    Assert-SameBytes (Join-Path $Root "LICENSE") $PackagedLicense "project license"
    Assert-SameBytes `
        (Join-Path $Root "THIRD_PARTY_NOTICES.md") `
        $PackagedNotices `
        "third-party notices"
    Assert-WindowsPeMachine $PackagedDesktop 0x8664 "packaged desktop"
    Assert-WindowsPeMachine $PackagedBridge 0x8664 "packaged MCP bridge"
    Assert-WindowsPeMachine $PackagedHelper 0x8664 "packaged firewall helper"
    Assert-WindowsFirewallHelperManifest `
        $PackagedHelper `
        "packaged Windows firewall helper"
    Assert-WindowsDirectoryTreeMatches `
        (Join-Path $Root "resources\licenses") `
        (Join-Path $PayloadRoot "licenses") `
        "packaged license inventory"
    $PackagedRuntimeRoot = Get-WindowsPackagedRuntimeRoot `
        $PackagedDesktop `
        $PackagedLlamaServer
    Assert-WindowsRuntimeTreeMatches `
        $LlamaRuntime `
        $PackagedRuntimeRoot
    $null = Get-WindowsLlamaRuntimeManifest $PackagedRuntimeRoot $LlamaPolicy
    Assert-WindowsDesktopEmbedsLlamaRuntimeHash `
        $PackagedDesktop `
        $PackagedRuntimeRoot `
        $LlamaPolicy

    $PackagedDesktopSigner = Assert-ExpectedWindowsSigner $PackagedDesktop
    $PackagedBridgeSigner = Assert-ExpectedWindowsSigner $PackagedBridge
    $PackagedHelperSigner = Assert-ExpectedWindowsSigner $PackagedHelper
    Assert-SameWindowsSigner $DesktopSigner $PackagedDesktopSigner "packaged desktop"
    Assert-SameWindowsSigner $DesktopSigner $PackagedBridgeSigner "packaged MCP bridge"
    Assert-SameWindowsSigner $DesktopSigner $PackagedHelperSigner "packaged firewall helper"

    Push-Location $Root
    try {
        $McpbArguments = @(
            "mcpb", "verify",
            "--target", "x86_64-pc-windows-msvc",
            "--bridge", $PackagedBridge,
            "--output", $PackagedMcpb
        )
        Invoke-ReleaseXtask $McpbArguments "packaged Claude MCPB failed validation"
    } finally {
        Pop-Location
    }
} finally {
    Remove-AirWikiWindowsStagingPath `
        -Path $ExtractDir `
        -AllowedRoot (Join-Path $Root "target") `
        -Label "signed Windows payload verification staging"
}

$SignaturePath = "$Installer.sig"
Push-Location $Root
try {
    $UpdaterArguments = @(
        "packaging", "verify-updater-signature",
        "--artifact", $Installer,
        "--signature", $SignaturePath
    )
    Invoke-ReleaseXtask $UpdaterArguments "updater signature failed cryptographic verification"
} finally {
    Pop-Location
}

Get-FileHash -LiteralPath $Installer -Algorithm SHA256
Get-FileHash -LiteralPath $SignaturePath -Algorithm SHA256
Get-FileHash -LiteralPath $UninstallerReceipt -Algorithm SHA256
