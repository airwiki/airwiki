param(
    [string] $NodeBinDir
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$TargetRoot = Join-Path $Root "target"
$OutDir = Join-Path $TargetRoot "packages\windows"
$ReleaseDir = Join-Path $Root "target\x86_64-pc-windows-msvc\release"
$Bridge = Join-Path $ReleaseDir "airwiki-mcp-bridge.exe"
$Desktop = Join-Path $ReleaseDir "airwiki.exe"
$FirewallHelper = Join-Path $ReleaseDir "airwiki-windows-firewall-helper.exe"
$Mcpb = Join-Path $Root "target\mcpb\x86_64-pc-windows-msvc\airwiki-claude.mcpb"
$Xtask = Join-Path $Root "target\debug\xtask.exe"
$Tauri = Join-Path $Root "apps\desktop\ui\node_modules\.bin\tauri.cmd"
$SvelteCheck = Join-Path $Root "apps\desktop\ui\node_modules\.bin\svelte-check.cmd"
$Vite = Join-Path $Root "apps\desktop\ui\node_modules\.bin\vite.cmd"
$TauriInstallerDir = Join-Path $ReleaseDir "bundle\msi"
$LlamaRuntime = Join-Path $Root "resources\llama\windows-x64"
$LlamaPolicy = Join-Path $Root "packaging\llama-windows-build-policy.json"
. (Join-Path $PSScriptRoot "windows-runtime.ps1")
. (Join-Path $PSScriptRoot "windows-payload.ps1")
. (Join-Path $PSScriptRoot "windows-safe-staging.ps1")
. (Join-Path $PSScriptRoot "windows-wix.ps1")

$PreviousPath = $env:Path
if ($NodeBinDir) {
    $ResolvedNodeBinDir = (Resolve-Path -LiteralPath $NodeBinDir).Path
    $NodeExecutable = Join-Path $ResolvedNodeBinDir "node.exe"
    $CorepackCli = Join-Path $ResolvedNodeBinDir "node_modules\corepack\dist\corepack.js"
    if (-not (Test-Path -LiteralPath $NodeExecutable -PathType Leaf) -or
        -not (Test-Path -LiteralPath $CorepackCli -PathType Leaf)) {
        throw "NodeBinDir must contain the official Node.js Windows distribution"
    }
    $NodeVersion = (& $NodeExecutable --version 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $NodeVersion -ne "v24.15.0") {
        throw "NodeBinDir must provide Node.js v24.15.0"
    }
    & $NodeExecutable $CorepackCli enable pnpm --install-directory $ResolvedNodeBinDir
    if ($LASTEXITCODE -ne 0) {
        throw "Could not provision the pinned pnpm shim"
    }
    $env:Path = "$ResolvedNodeBinDir;$PreviousPath"
}

function Assert-X64Pe([string] $Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Missing fresh executable: $Path"
    }
    $Bytes = [System.IO.File]::ReadAllBytes($Path)
    if ($Bytes.Length -lt 64 -or $Bytes[0] -ne 0x4d -or $Bytes[1] -ne 0x5a) {
        throw "Executable is not a PE file: $Path"
    }
    $Offset = [BitConverter]::ToUInt32($Bytes, 0x3c)
    if ($Offset + 6 -gt $Bytes.Length) {
        throw "Executable has a truncated PE header: $Path"
    }
    if ($Bytes[$Offset] -ne 0x50 -or $Bytes[$Offset + 1] -ne 0x45 -or
        $Bytes[$Offset + 2] -ne 0 -or $Bytes[$Offset + 3] -ne 0 -or
        $Bytes[$Offset + 4] -ne 0x64 -or $Bytes[$Offset + 5] -ne 0x86) {
        throw "Executable is not Windows x64: $Path"
    }
}

function Get-SinglePayload([string] $Root, [string] $Name) {
    $Matches = @(Get-ChildItem -LiteralPath $Root -Recurse -File -Filter $Name)
    if ($Matches.Count -ne 1) {
        throw "Expected exactly one $Name in the MSI payload"
    }
    return $Matches[0].FullName
}

function Assert-SameBytes([string] $Expected, [string] $Actual, [string] $Label) {
    $ExpectedHash = (Get-FileHash -LiteralPath $Expected -Algorithm SHA256).Hash
    $ActualHash = (Get-FileHash -LiteralPath $Actual -Algorithm SHA256).Hash
    if ($ExpectedHash -ne $ActualHash) {
        throw "$Label in the MSI payload differs from the fresh artifact"
    }
}

function Assert-WindowsMsi([string] $Path) {
    $Verified = Get-VerifiedWindowsRegularFile $Path "fresh MSI installer"
    $Bytes = [IO.File]::ReadAllBytes($Verified)
    $OleHeader = [byte[]] @(0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1)
    if ($Bytes.Length -lt $OleHeader.Length) {
        throw "MSI installer is truncated"
    }
    for ($Index = 0; $Index -lt $OleHeader.Length; $Index++) {
        if ($Bytes[$Index] -ne $OleHeader[$Index]) {
            throw "Installer is not an MSI compound file"
        }
    }
}

function Expand-WindowsMsi([string] $Installer, [string] $Destination) {
    $MsiExec = Join-Path $env:SystemRoot "System32\msiexec.exe"
    $VerifiedMsiExec = Get-VerifiedWindowsRegularFile $MsiExec "Windows Installer executable"
    $Arguments = "/a `"$Installer`" /qn /norestart TARGETDIR=`"$Destination`""
    $Process = Start-Process `
        -FilePath $VerifiedMsiExec `
        -ArgumentList $Arguments `
        -Wait `
        -PassThru `
        -WindowStyle Hidden
    if ($Process.ExitCode -ne 0) {
        throw "Windows Installer could not extract the MSI payload (exit $($Process.ExitCode))"
    }
}

function Assert-WindowsMsiPayload(
    [string] $Installer,
    [string] $ExtractDir,
    [string] $Desktop,
    [string] $Bridge,
    [string] $FirewallHelper,
    [string] $Mcpb,
    [string] $LlamaRuntime,
    [string] $LlamaPolicy,
    [string] $Xtask,
    [string] $Root
) {
    Remove-AirWikiWindowsStagingPath `
        -Path $ExtractDir `
        -AllowedRoot (Join-Path $Root "target") `
        -Label "Windows payload verification staging"
    New-Item -ItemType Directory -Path $ExtractDir -Force | Out-Null
    try {
        Expand-WindowsMsi $Installer $ExtractDir
        $PackagedDesktop = Get-SinglePayload $ExtractDir "airwiki.exe"
        $PayloadRoot = [IO.Path]::GetDirectoryName($PackagedDesktop)
        $PackagedBridge = Get-VerifiedWindowsRegularFile `
            (Join-Path $PayloadRoot "integrations\bridge\airwiki-mcp-bridge.exe") `
            "packaged MCP bridge"
        $PackagedFirewallHelper = Get-VerifiedWindowsRegularFile `
            (Join-Path $PayloadRoot "airwiki-windows-firewall-helper.exe") `
            "packaged firewall helper"
        $PackagedMcpb = Get-VerifiedWindowsRegularFile `
            (Join-Path $PayloadRoot "integrations\airwiki-claude.mcpb") `
            "packaged Claude MCPB"
        $PackagedLlamaServer = Get-SinglePayload $ExtractDir "llama-server.exe"
        $PackagedLicense = Get-VerifiedWindowsRegularFile `
            (Join-Path $PayloadRoot "LICENSE") `
            "packaged project license"
        $PackagedNotices = Get-VerifiedWindowsRegularFile `
            (Join-Path $PayloadRoot "THIRD_PARTY_NOTICES.md") `
            "packaged third-party notices"
        Assert-X64Pe $PackagedDesktop
        Assert-X64Pe $PackagedBridge
        Assert-X64Pe $PackagedFirewallHelper
        Assert-WindowsFirewallHelperManifest `
            $PackagedFirewallHelper `
            "packaged Windows firewall helper"
        Assert-WindowsMsiBundleTypePatch `
            $Desktop `
            $PackagedDesktop `
            "Desktop executable"
        Assert-SameBytes $Bridge $PackagedBridge "MCP bridge"
        Assert-SameBytes $FirewallHelper $PackagedFirewallHelper "Windows Firewall helper"
        Assert-SameBytes $Mcpb $PackagedMcpb "Claude MCPB"
        Assert-SameBytes (Join-Path $Root "LICENSE") $PackagedLicense "project license"
        Assert-SameBytes `
            (Join-Path $Root "THIRD_PARTY_NOTICES.md") `
            $PackagedNotices `
            "third-party notices"
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
        & $Xtask mcpb verify `
            --target x86_64-pc-windows-msvc `
            --bridge $PackagedBridge `
            --output $PackagedMcpb
        if ($LASTEXITCODE -ne 0) {
            throw "Claude MCPB inside the MSI payload failed validation"
        }
    } finally {
        Remove-AirWikiWindowsStagingPath `
            -Path $ExtractDir `
            -AllowedRoot (Join-Path $Root "target") `
            -Label "Windows payload verification staging"
    }
}

Push-Location $Root
try {
    foreach ($FrontendTool in @($Tauri, $SvelteCheck, $Vite)) {
        if (-not (Test-Path -LiteralPath $FrontendTool -PathType Leaf)) {
            throw "pinned frontend build dependencies are missing; run pnpm install --frozen-lockfile --ignore-scripts --prod=false"
        }
    }
    $TauriVersion = (& $Tauri --version 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $TauriVersion -ne "tauri-cli 2.11.4") {
        throw "Tauri CLI 2.11.4 is required"
    }
    Assert-AirWikiWindowsPathHasNoReparsePoint `
        $TargetRoot `
        "Windows target staging root"
    New-Item -ItemType Directory -Path $TargetRoot -Force | Out-Null
    Assert-AirWikiWindowsPathHasNoReparsePoint `
        $TargetRoot `
        "Windows target staging root"
    Remove-AirWikiWindowsStagingPath `
        -Path $OutDir `
        -AllowedRoot $TargetRoot `
        -Label "Windows package output"
    New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
    $Started = [DateTime]::UtcNow

    & cargo build --locked -p xtask
    if ($LASTEXITCODE -ne 0) {
        throw "xtask build failed"
    }
    & $Xtask licenses check
    if ($LASTEXITCODE -ne 0) {
        throw "license validation failed"
    }
    & $Xtask packaging verify-windows-msi
    if ($LASTEXITCODE -ne 0) {
        throw "Windows MSI policy validation failed"
    }
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File packaging\fetch-llama-windows.ps1
    if ($LASTEXITCODE -ne 0) {
        throw "llama.cpp runtime source build failed"
    }
    $LlamaManifest = Get-WindowsLlamaRuntimeManifest $LlamaRuntime $LlamaPolicy
    $env:AIRWIKI_WINDOWS_LLAMA_SERVER_SHA256 = `
        [string] @($LlamaManifest.runtime.files)[0].sha256
    & cargo build --locked --release --target x86_64-pc-windows-msvc `
        -p airwiki-mcp-bridge `
        -p airwiki-windows-firewall-helper
    if ($LASTEXITCODE -ne 0) {
        throw "release build failed"
    }
    & $Xtask mcpb build `
        --target x86_64-pc-windows-msvc `
        --bridge $Bridge `
        --output $Mcpb
    if ($LASTEXITCODE -ne 0) {
        throw "Claude MCPB build failed"
    }
    Assert-X64Pe $Bridge
    Assert-X64Pe $FirewallHelper
    Assert-WindowsFirewallHelperManifest `
        $FirewallHelper `
        "fresh Windows firewall helper"
    & $Xtask mcpb verify `
        --target x86_64-pc-windows-msvc `
        --bridge $Bridge `
        --output $Mcpb
    if ($LASTEXITCODE -ne 0) {
        throw "Claude MCPB validation failed"
    }

    Push-Location (Join-Path $Root "apps\desktop")
    try {
        & $Tauri build `
            --ci `
            --config ..\..\packaging\windows\tauri.msi.bundle.conf.json `
            --target x86_64-pc-windows-msvc `
            --bundles msi
        if ($LASTEXITCODE -ne 0) {
            Write-WixLightDiagnostic $Root $ReleaseDir
            throw "Tauri MSI packaging failed"
        }
    } finally {
        Pop-Location
    }
    Assert-X64Pe $Desktop
    Assert-WindowsDesktopEmbedsLlamaRuntimeHash $Desktop $LlamaRuntime $LlamaPolicy

    $TauriInstallers = @(Get-ChildItem -LiteralPath $TauriInstallerDir -File -Filter *.msi)
    if ($TauriInstallers.Count -ne 2) {
        throw "Expected exactly two localized Tauri MSI installers"
    }
    foreach ($TauriInstaller in $TauriInstallers) {
        Copy-Item -LiteralPath $TauriInstaller.FullName -Destination $OutDir
    }
    $Installers = @(Get-ChildItem -LiteralPath $OutDir -File -Filter *.msi)
    if ($Installers.Count -ne 2) {
        throw "Expected exactly two fresh localized MSI installers"
    }
    foreach ($Installer in $Installers) {
        if ($Installer.LastWriteTimeUtc -lt $Started) {
            throw "MSI installer predates this packaging run"
        }
        Assert-WindowsMsi $Installer.FullName
    }

    for ($Index = 0; $Index -lt $Installers.Count; $Index++) {
        Assert-WindowsMsiPayload `
            -Installer $Installers[$Index].FullName `
            -ExtractDir (Join-Path $Root "target\packages\windows-payload-check-$Index") `
            -Desktop $Desktop `
            -Bridge $Bridge `
            -FirewallHelper $FirewallHelper `
            -Mcpb $Mcpb `
            -LlamaRuntime $LlamaRuntime `
            -LlamaPolicy $LlamaPolicy `
            -Xtask $Xtask `
            -Root $Root
    }
    Write-Host "Verified fresh Windows x64 MSI installers: $($Installers.FullName -join ', ')"
} finally {
    $env:Path = $PreviousPath
    Pop-Location
}
