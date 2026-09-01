[CmdletBinding()]
param(
    [string] $OutputDirectory = "target\signpath\windows-binaries"
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    throw "SignPath binary preparation requires Windows"
}

$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$TargetRoot = Join-Path $Root "target"
$ReleaseRoot = Join-Path $TargetRoot "x86_64-pc-windows-msvc\release"
$OutputRoot = if ([IO.Path]::IsPathRooted($OutputDirectory)) {
    [IO.Path]::GetFullPath($OutputDirectory)
} else {
    [IO.Path]::GetFullPath((Join-Path $Root $OutputDirectory))
}

. (Join-Path $PSScriptRoot "windows-runtime.ps1")
. (Join-Path $PSScriptRoot "windows-payload.ps1")
. (Join-Path $PSScriptRoot "windows-safe-staging.ps1")

$TargetBoundary = [IO.Path]::GetFullPath($TargetRoot).TrimEnd('\') + '\'
if (-not $OutputRoot.StartsWith($TargetBoundary, [StringComparison]::OrdinalIgnoreCase)) {
    throw "SignPath staging must remain below the repository target directory"
}

Push-Location $Root
try {
    & (Join-Path $PSScriptRoot "prepare-windows-release.ps1")
    if ($LASTEXITCODE -ne 0) {
        throw "Windows release preparation failed"
    }

    $Desktop = Get-VerifiedWindowsRegularFile `
        (Join-Path $ReleaseRoot "airwiki.exe") `
        "unsigned Windows desktop"
    $Bridge = Get-VerifiedWindowsRegularFile `
        (Join-Path $ReleaseRoot "airwiki-mcp-bridge.exe") `
        "unsigned Windows MCP bridge"
    $Helper = Get-VerifiedWindowsRegularFile `
        (Join-Path $ReleaseRoot "airwiki-windows-firewall-helper.exe") `
        "unsigned Windows firewall helper"

    foreach ($Artifact in @($Desktop, $Bridge, $Helper)) {
        $Signature = Get-AuthenticodeSignature -LiteralPath $Artifact
        if ($Signature.Status -ne [System.Management.Automation.SignatureStatus]::NotSigned) {
            throw "SignPath input binaries must be unsigned"
        }
    }

    Set-WindowsMsiBundleType $Desktop
    Remove-AirWikiWindowsStagingPath `
        -Path $OutputRoot `
        -AllowedRoot $TargetRoot `
        -Label "SignPath Windows binary staging"
    New-Item -ItemType Directory -Path $OutputRoot -Force | Out-Null
    foreach ($Artifact in @($Desktop, $Bridge, $Helper)) {
        Copy-Item -LiteralPath $Artifact -Destination $OutputRoot
    }

    $Files = @(Get-ChildItem -LiteralPath $OutputRoot -File)
    if ($Files.Count -ne 3) {
        throw "SignPath binary staging must contain exactly three executables"
    }
    foreach ($File in $Files) {
        Assert-WindowsPeMachine $File.FullName 0x8664 "SignPath binary input"
    }
    Write-Host "Prepared SignPath binary input: $OutputRoot"
} finally {
    Pop-Location
}

