$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$OutDir = Join-Path $Root "target\packages\windows"
$ReleaseDir = Join-Path $Root "target\x86_64-pc-windows-msvc\release"
$Bridge = Join-Path $ReleaseDir "airwiki-mcp-bridge.exe"
$Desktop = Join-Path $ReleaseDir "airwiki.exe"
$FirewallHelper = Join-Path $ReleaseDir "airwiki-windows-firewall-helper.exe"
$Mcpb = Join-Path $Root "target\mcpb\x86_64-pc-windows-msvc\airwiki-claude.mcpb"
$Xtask = Join-Path $Root "target\debug\xtask.exe"
$NsisToolCacheRoot = Join-Path ([Environment]::GetFolderPath("LocalApplicationData")) ".cargo-packager"
$SevenZipToolRoot = Join-Path $Root "target\verified-tools\7zip-26.02"
$SevenZip = Join-Path $SevenZipToolRoot "7z.exe"
$LlamaRuntime = Join-Path $Root "resources\llama\windows-x64"
$LlamaPolicy = Join-Path $Root "packaging\llama-windows-build-policy.json"
. (Join-Path $PSScriptRoot "windows-runtime.ps1")
. (Join-Path $PSScriptRoot "windows-payload.ps1")
. (Join-Path $PSScriptRoot "windows-safe-staging.ps1")

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
        throw "Expected exactly one $Name in the NSIS payload"
    }
    return $Matches[0].FullName
}

function Assert-SameBytes([string] $Expected, [string] $Actual, [string] $Label) {
    $ExpectedHash = (Get-FileHash -LiteralPath $Expected -Algorithm SHA256).Hash
    $ActualHash = (Get-FileHash -LiteralPath $Actual -Algorithm SHA256).Hash
    if ($ExpectedHash -ne $ActualHash) {
        throw "$Label in the NSIS payload differs from the fresh artifact"
    }
}

Push-Location $Root
try {
    $CargoPackager = (Get-Command cargo-packager.exe -CommandType Application `
        -ErrorAction Stop).Source
    $CargoPackagerVersion = (& $CargoPackager --version 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $CargoPackagerVersion -ne "cargo-packager 0.11.8") {
        throw "cargo-packager 0.11.8 is required"
    }
    & (Join-Path $PSScriptRoot "prepare-verified-nsis-toolchain.ps1") `
        -ToolCacheRoot $NsisToolCacheRoot | Out-Null
    & (Join-Path $PSScriptRoot "prepare-verified-7zip.ps1") `
        -ToolRoot $SevenZipToolRoot | Out-Null
    Remove-AirWikiWindowsStagingPath `
        -Path $OutDir `
        -AllowedRoot (Join-Path $Root "target") `
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
    & $Xtask packaging verify-windows-uninstaller
    if ($LASTEXITCODE -ne 0) {
        throw "Windows uninstaller policy validation failed"
    }
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File packaging\fetch-llama-windows.ps1
    if ($LASTEXITCODE -ne 0) {
        throw "llama.cpp runtime source build failed"
    }
    $LlamaManifest = Get-WindowsLlamaRuntimeManifest $LlamaRuntime $LlamaPolicy
    $env:AIRWIKI_WINDOWS_LLAMA_SERVER_SHA256 = `
        [string] @($LlamaManifest.runtime.files)[0].sha256
    & cargo build --locked --release --target x86_64-pc-windows-msvc `
        -p airwiki-desktop `
        -p airwiki-mcp-bridge `
        -p airwiki-windows-firewall-helper
    if ($LASTEXITCODE -ne 0) {
        throw "release build failed"
    }
    Assert-WindowsDesktopEmbedsLlamaRuntimeHash $Desktop $LlamaRuntime $LlamaPolicy
    & $Xtask mcpb build `
        --target x86_64-pc-windows-msvc `
        --bridge $Bridge `
        --output $Mcpb
    if ($LASTEXITCODE -ne 0) {
        throw "Claude MCPB build failed"
    }
    Assert-X64Pe $Desktop
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

    # The runtime is built before the desktop so its per-candidate SHA-256 is
    # embedded in the executable. Revalidate both after every Cargo build.
    Assert-WindowsDesktopEmbedsLlamaRuntimeHash $Desktop $LlamaRuntime $LlamaPolicy
    & $CargoPackager --config packaging/windows/Packager.toml
    if ($LASTEXITCODE -ne 0) {
        throw "cargo-packager failed"
    }

    $Installers = @(Get-ChildItem -LiteralPath $OutDir -File -Filter *.exe)
    if ($Installers.Count -ne 1) {
        throw "Expected exactly one fresh NSIS installer"
    }
    if ($Installers[0].LastWriteTimeUtc -lt $Started) {
        throw "NSIS installer predates this packaging run"
    }
    Assert-WindowsPeMachine $Installers[0].FullName 0x014c "fresh NSIS installer"

    $ExtractDir = Join-Path $Root "target\packages\windows-payload-check"
    Remove-AirWikiWindowsStagingPath `
        -Path $ExtractDir `
        -AllowedRoot (Join-Path $Root "target") `
        -Label "Windows payload verification staging"
    New-Item -ItemType Directory -Path $ExtractDir -Force | Out-Null
    try {
        & $SevenZip x -y "-o$ExtractDir" $Installers[0].FullName | Out-Null
        if ($LASTEXITCODE -ne 0) {
            throw "7-Zip could not inspect the NSIS payload"
        }
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
        Assert-SameBytes $Desktop $PackagedDesktop "Desktop executable"
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
            throw "Claude MCPB inside the NSIS payload failed validation"
        }
    } finally {
        Remove-AirWikiWindowsStagingPath `
            -Path $ExtractDir `
            -AllowedRoot (Join-Path $Root "target") `
            -Label "Windows payload verification staging"
    }
    Write-Host "Verified fresh Windows x64 installer: $($Installers[0].FullName)"
} finally {
    Pop-Location
}
