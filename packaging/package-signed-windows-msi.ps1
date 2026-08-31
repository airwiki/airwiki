[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $SignedBinaryRoot,

    [string] $OutputDirectory = "target\windows-signing\windows-msi",

    [string] $ExpectedVersion = $env:AIRWIKI_RELEASE_VERSION
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    throw "Signed Windows MSI packaging requires Windows"
}

$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
. (Join-Path $PSScriptRoot "windows-release-version.ps1")
$ReleaseVersion = Get-AirWikiReleaseVersion $Root $ExpectedVersion
$TargetRoot = Join-Path $Root "target"
$ReleaseRoot = Join-Path $TargetRoot "x86_64-pc-windows-msvc\release"
$BundleRoot = Join-Path $ReleaseRoot "bundle\msi"
$Mcpb = Join-Path $TargetRoot "mcpb\x86_64-pc-windows-msvc\airwiki-claude.mcpb"
$LlamaRuntime = Join-Path $Root "resources\llama\windows-x64"
$LlamaPolicy = Join-Path $Root "packaging\llama-windows-build-policy.json"
$Tauri = Join-Path $Root "apps\desktop\ui\node_modules\.bin\tauri.cmd"
$OutputRoot = if ([IO.Path]::IsPathRooted($OutputDirectory)) {
    [IO.Path]::GetFullPath($OutputDirectory)
} else {
    [IO.Path]::GetFullPath((Join-Path $Root $OutputDirectory))
}
$CandidateSignedRoot = if ([IO.Path]::IsPathRooted($SignedBinaryRoot)) {
    [IO.Path]::GetFullPath($SignedBinaryRoot)
} else {
    [IO.Path]::GetFullPath((Join-Path (Get-Location).Path $SignedBinaryRoot))
}

. (Join-Path $PSScriptRoot "windows-payload.ps1")
. (Join-Path $PSScriptRoot "windows-runtime.ps1")
. (Join-Path $PSScriptRoot "windows-safe-staging.ps1")
. (Join-Path $PSScriptRoot "windows-authenticode.ps1")
. (Join-Path $PSScriptRoot "windows-wix.ps1")

function Assert-TargetDescendant([string] $Path, [string] $Label) {
    $Boundary = [IO.Path]::GetFullPath($TargetRoot).TrimEnd('\') + '\'
    if (-not $Path.StartsWith($Boundary, [StringComparison]::OrdinalIgnoreCase)) {
        throw "$Label must remain below the repository target directory"
    }
}

function Assert-SameBytes([string] $Expected, [string] $Actual, [string] $Label) {
    $ExpectedHash = (Get-FileHash -LiteralPath $Expected -Algorithm SHA256).Hash
    $ActualHash = (Get-FileHash -LiteralPath $Actual -Algorithm SHA256).Hash
    if ($ExpectedHash -ne $ActualHash) {
        throw "$Label differs from the signed source artifact"
    }
}

function Expand-WindowsMsi([string] $Installer, [string] $Destination) {
    $MsiExec = Get-VerifiedWindowsRegularFile `
        (Join-Path $env:SystemRoot "System32\msiexec.exe") `
        "Windows Installer executable"
    $Arguments = "/a `"$Installer`" /qn /norestart TARGETDIR=`"$Destination`""
    $Process = Start-Process `
        -FilePath $MsiExec `
        -ArgumentList $Arguments `
        -Wait `
        -PassThru `
        -WindowStyle Hidden
    if ($Process.ExitCode -ne 0) {
        throw "Windows Installer could not extract the MSI payload (exit $($Process.ExitCode))"
    }
}

Assert-TargetDescendant $OutputRoot "Windows signing MSI staging"
Assert-TargetDescendant $CandidateSignedRoot "Windows signing signed-binary input"
if (-not (Test-Path -LiteralPath $CandidateSignedRoot -PathType Container)) {
    throw "Windows signing signed-binary input is missing"
}
Assert-NoWindowsReparseAncestor $CandidateSignedRoot "Windows signing signed-binary input"

$ExpectedNames = @(
    "airwiki.exe",
    "airwiki-mcp-bridge.exe",
    "airwiki-windows-firewall-helper.exe"
)
$SignedFiles = @(Get-ChildItem -LiteralPath $CandidateSignedRoot -File)
if ($SignedFiles.Count -ne $ExpectedNames.Count) {
    throw "Windows signing signed-binary input must contain exactly three executables"
}
foreach ($File in $SignedFiles) {
    if ($File.Name -cnotin $ExpectedNames) {
        throw "Windows signing signed-binary input contains an unexpected file"
    }
}

$SignedDesktop = Get-VerifiedWindowsRegularFile `
    (Join-Path $CandidateSignedRoot "airwiki.exe") `
    "signed Windows desktop"
$SignedBridge = Get-VerifiedWindowsRegularFile `
    (Join-Path $CandidateSignedRoot "airwiki-mcp-bridge.exe") `
    "signed Windows MCP bridge"
$SignedHelper = Get-VerifiedWindowsRegularFile `
    (Join-Path $CandidateSignedRoot "airwiki-windows-firewall-helper.exe") `
    "signed Windows firewall helper"
$DesktopSigner = Get-VerifiedWindowsAuthenticodeSignature $SignedDesktop "signed Windows desktop"
$BridgeSigner = Get-VerifiedWindowsAuthenticodeSignature $SignedBridge "signed Windows MCP bridge"
$HelperSigner = Get-VerifiedWindowsAuthenticodeSignature $SignedHelper "signed Windows firewall helper"
Assert-ExpectedWindowsSigner $DesktopSigner
Assert-ExpectedWindowsSigner $BridgeSigner
Assert-ExpectedWindowsSigner $HelperSigner
Assert-SameWindowsSigner $DesktopSigner $BridgeSigner "MCP bridge"
Assert-SameWindowsSigner $DesktopSigner $HelperSigner "firewall helper"
Assert-WindowsMsiBundleType $SignedDesktop "signed Windows desktop"
Assert-WindowsFirewallHelperManifest $SignedHelper "signed Windows firewall helper"

Push-Location $Root
try {
    & cargo run --locked -p xtask -- licenses check
    if ($LASTEXITCODE -ne 0) { throw "license validation failed" }
    & cargo run --locked -p xtask -- packaging verify-windows-msi
    if ($LASTEXITCODE -ne 0) { throw "Windows MSI policy validation failed" }
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File packaging\fetch-llama-windows.ps1
    if ($LASTEXITCODE -ne 0) { throw "llama.cpp runtime source build failed" }

    $null = Get-WindowsLlamaRuntimeManifest $LlamaRuntime $LlamaPolicy
    Assert-WindowsDesktopEmbedsLlamaRuntimeHash $SignedDesktop $LlamaRuntime $LlamaPolicy
    New-Item -ItemType Directory -Path $ReleaseRoot -Force | Out-Null
    Copy-Item -LiteralPath $SignedDesktop -Destination (Join-Path $ReleaseRoot "airwiki.exe") -Force
    Copy-Item -LiteralPath $SignedBridge -Destination (Join-Path $ReleaseRoot "airwiki-mcp-bridge.exe") -Force
    Copy-Item -LiteralPath $SignedHelper -Destination (Join-Path $ReleaseRoot "airwiki-windows-firewall-helper.exe") -Force

    & cargo run --locked -p xtask -- mcpb build `
        --target x86_64-pc-windows-msvc `
        --bridge (Join-Path $ReleaseRoot "airwiki-mcp-bridge.exe") `
        --output $Mcpb
    if ($LASTEXITCODE -ne 0) { throw "Claude MCPB build failed" }
    & pnpm.cmd --dir apps\desktop\ui run build
    if ($LASTEXITCODE -ne 0) { throw "frontend build failed" }
    & cargo run --locked -p xtask -- packaging generate-windows-msi-resources
    if ($LASTEXITCODE -ne 0) { throw "Windows MSI resource fragment generation failed" }

    $Tauri = Get-VerifiedWindowsRegularFile $Tauri "pinned Tauri CLI"
    $TauriVersion = (& $Tauri --version 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $TauriVersion -ne "tauri-cli 2.11.4") {
        throw "Tauri CLI 2.11.4 is required"
    }
    Remove-AirWikiWindowsStagingPath `
        -Path $BundleRoot `
        -AllowedRoot $TargetRoot `
        -Label "Tauri MSI output"
    $BundleStarted = [DateTime]::UtcNow
    Push-Location (Join-Path $Root "apps\desktop")
    try {
        & $Tauri bundle `
            --ci `
            --config ..\..\packaging\windows\tauri.msi.bundle.conf.json `
            --target x86_64-pc-windows-msvc `
            --bundles msi
        if ($LASTEXITCODE -ne 0) {
            Write-WixLightDiagnostic $Root $ReleaseRoot
            throw "Tauri MSI packaging failed"
        }
    } finally {
        Pop-Location
    }
    Assert-WindowsWixLicenseRtf $ReleaseRoot $BundleStarted

    $Installers = @(Get-ChildItem -LiteralPath $BundleRoot -File -Filter *.msi)
    $ExpectedInstallerNames = @(
        "AirWiki_${ReleaseVersion}_x64_en-US.msi",
        "AirWiki_${ReleaseVersion}_x64_es-ES.msi"
    )
    if ($Installers.Count -ne 2 -or
        @($Installers | Where-Object { $_.Name -cnotin $ExpectedInstallerNames }).Count -ne 0) {
        throw "Tauri must produce the exact two versioned localized MSI installers"
    }
    Remove-AirWikiWindowsStagingPath `
        -Path $OutputRoot `
        -AllowedRoot $TargetRoot `
        -Label "Windows signing MSI staging"
    New-Item -ItemType Directory -Path $OutputRoot -Force | Out-Null
    foreach ($Installer in $Installers) {
        $Signature = Get-AuthenticodeSignature -LiteralPath $Installer.FullName
        if ($Signature.Status -ne [System.Management.Automation.SignatureStatus]::NotSigned) {
            throw "MSI submitted for signing must not already have an outer signature"
        }
        Copy-Item -LiteralPath $Installer.FullName -Destination $OutputRoot
    }

    $VerificationRoot = Join-Path $TargetRoot "windows-signing\windows-msi-payload-check"
    foreach ($Installer in @(Get-ChildItem -LiteralPath $OutputRoot -File -Filter *.msi)) {
        Remove-AirWikiWindowsStagingPath `
            -Path $VerificationRoot `
            -AllowedRoot $TargetRoot `
            -Label "MSI payload verification staging"
        New-Item -ItemType Directory -Path $VerificationRoot -Force | Out-Null
        Expand-WindowsMsi $Installer.FullName $VerificationRoot
        $PackagedDesktop = @(Get-ChildItem -LiteralPath $VerificationRoot -Recurse -File -Filter airwiki.exe)
        if ($PackagedDesktop.Count -ne 1) { throw "MSI must contain exactly one AirWiki desktop" }
        $PayloadRoot = $PackagedDesktop[0].Directory.FullName
        $PackagedBridge = Get-VerifiedWindowsRegularFile `
            (Join-Path $PayloadRoot "integrations\bridge\airwiki-mcp-bridge.exe") `
            "MSI MCP bridge"
        $PackagedHelper = Get-VerifiedWindowsRegularFile `
            (Join-Path $PayloadRoot "airwiki-windows-firewall-helper.exe") `
            "MSI firewall helper"
        $PackagedMcpb = Get-VerifiedWindowsRegularFile `
            (Join-Path $PayloadRoot "integrations\airwiki-claude.mcpb") `
            "MSI Claude MCPB"
        Assert-SameBytes $SignedDesktop $PackagedDesktop[0].FullName "MSI desktop"
        Assert-SameBytes $SignedBridge $PackagedBridge "MSI MCP bridge"
        Assert-SameBytes $SignedHelper $PackagedHelper "MSI firewall helper"
        Assert-WindowsDirectoryTreeMatches `
            (Join-Path $Root "resources\integrations\workflow") `
            (Join-Path $PayloadRoot "integrations\workflow") `
            "MSI AirWiki workflow guide"
        & cargo run --locked -p xtask -- mcpb verify `
            --target x86_64-pc-windows-msvc `
            --bridge $PackagedBridge `
            --output $PackagedMcpb
        if ($LASTEXITCODE -ne 0) { throw "MSI Claude MCPB failed validation" }
    }
    Remove-AirWikiWindowsStagingPath `
        -Path $VerificationRoot `
        -AllowedRoot $TargetRoot `
        -Label "MSI payload verification staging"
    Write-Host "Prepared MSI artifacts for Windows signing: $OutputRoot"
} finally {
    Pop-Location
}

