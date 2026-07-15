[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $ToolCacheRoot,

    [string] $ArtifactDirectory
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

. (Join-Path $PSScriptRoot "windows-safe-staging.ps1")

$PinnedArtifacts = @(
    @{
        Name = "nsis-3.09.zip"
        Url = "https://github.com/tauri-apps/binary-releases/releases/download/nsis-3.9/nsis-3.09.zip"
        Sha256 = "f5dc52eef1f3884230520199bac6f36b82d643d86b003ce51bd24b05c6ba7c91"
    },
    @{
        Name = "nsis_tauri_utils.dll"
        Url = "https://github.com/tauri-apps/nsis-tauri-utils/releases/download/nsis_tauri_utils-v0.2.1/nsis_tauri_utils.dll"
        Sha256 = "0eed48313a7f904d7cc1977b70000ab3f11f18cadc8e6a69b807d288ca71f9db"
    }
)

function Assert-RegularFile([string] $Path, [string] $Label) {
    $Item = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
    if ($Item.PSIsContainer -or ($Item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "$Label must be a regular file"
    }
}

function Get-PinnedArtifact(
    [hashtable] $Artifact,
    [string] $DestinationDirectory,
    [string] $LocalArtifactDirectory
) {
    $Destination = Join-Path $DestinationDirectory $Artifact.Name
    if ([string]::IsNullOrWhiteSpace($LocalArtifactDirectory)) {
        Invoke-WebRequest -UseBasicParsing -Uri $Artifact.Url -OutFile $Destination
    } else {
        $Source = Join-Path $LocalArtifactDirectory $Artifact.Name
        Assert-RegularFile $Source "Pinned NSIS artifact"
        Copy-Item -LiteralPath $Source -Destination $Destination
    }

    Assert-RegularFile $Destination "Downloaded NSIS artifact"
    $ActualHash = (Get-FileHash -LiteralPath $Destination -Algorithm SHA256).Hash
    if (-not $ActualHash.Equals($Artifact.Sha256, [StringComparison]::OrdinalIgnoreCase)) {
        throw "SHA-256 mismatch for $($Artifact.Name)"
    }
    return $Destination
}

function Assert-RequiredNsisLayout([string] $NsisRoot) {
    $RequiredFiles = @(
        "makensis.exe",
        "Bin\makensis.exe",
        "Stubs\lzma-x86-unicode",
        "Stubs\lzma_solid-x86-unicode",
        "Plugins\x86-unicode\ApplicationID.dll",
        "Plugins\x86-unicode\nsis_tauri_utils.dll",
        "Include\MUI2.nsh",
        "Include\FileFunc.nsh",
        "Include\x64.nsh",
        "Include\nsDialogs.nsh",
        "Include\WinMessages.nsh"
    )
    foreach ($RelativePath in $RequiredFiles) {
        Assert-RegularFile (Join-Path $NsisRoot $RelativePath) "Prepared NSIS toolchain entry"
    }
    $Sentinel = Join-Path $NsisRoot "Plugins\x86-unicode\ApplicationID.dll"
    $SentinelItem = Get-Item -LiteralPath $Sentinel
    $SentinelHash = (Get-FileHash -LiteralPath $Sentinel -Algorithm SHA256).Hash
    if ($SentinelItem.Length -ne 0 -or -not $SentinelHash.Equals(
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        [StringComparison]::OrdinalIgnoreCase
    )) {
        throw "cargo-packager ApplicationID compatibility sentinel is not empty"
    }
}

$ResolvedToolCacheRoot = [IO.Path]::GetFullPath($ToolCacheRoot)
$ResolvedArtifactDirectory = if ([string]::IsNullOrWhiteSpace($ArtifactDirectory)) {
    ""
} else {
    (Resolve-Path -LiteralPath $ArtifactDirectory -ErrorAction Stop).Path
}
$Scratch = Join-Path ([IO.Path]::GetTempPath()) "airwiki-nsis-$([Guid]::NewGuid().ToString('N'))"
$ToolCacheParent = Split-Path -Parent $ResolvedToolCacheRoot
if ([string]::IsNullOrWhiteSpace($ToolCacheParent)) {
    throw "Pinned NSIS tool cache must have a parent directory"
}
New-Item -ItemType Directory -Path $ToolCacheParent -Force | Out-Null
$Stage = Join-Path $ToolCacheParent ".airwiki-nsis-stage-$([Guid]::NewGuid().ToString('N'))"

try {
    New-Item -ItemType Directory -Path $Scratch, $Stage -Force | Out-Null
    $Downloaded = @{}
    foreach ($Artifact in $PinnedArtifacts) {
        $Downloaded[$Artifact.Name] = Get-PinnedArtifact $Artifact $Scratch $ResolvedArtifactDirectory
    }

    $NsisExtract = Join-Path $Scratch "nsis-extract"
    New-Item -ItemType Directory -Path $NsisExtract | Out-Null
    Expand-Archive -LiteralPath $Downloaded["nsis-3.09.zip"] -DestinationPath $NsisExtract

    $ExtractedNsis = Join-Path $NsisExtract "nsis-3.09"
    if (-not (Test-Path -LiteralPath $ExtractedNsis -PathType Container)) {
        throw "Pinned NSIS archive has an unexpected layout"
    }
    $PreparedNsis = Join-Path $Stage "NSIS"
    Move-Item -LiteralPath $ExtractedNsis -Destination $PreparedNsis
    $UnicodePlugins = Join-Path $PreparedNsis "Plugins\x86-unicode"
    New-Item -ItemType Directory -Path $UnicodePlugins -Force | Out-Null
    # cargo-packager 0.11.8 hard-codes this compatibility path as required even
    # when the managed template does not invoke the plug-in. The upstream
    # ApplicationID binary has no verified redistribution license, so create a
    # zero-byte non-executable sentinel. Any command using that namespace
    # remains a compile error and is rejected earlier by the legal gate.
    $CompatibilitySentinel = Join-Path $UnicodePlugins "ApplicationID.dll"
    [IO.File]::WriteAllBytes($CompatibilitySentinel, [byte[]] @())
    Copy-Item -LiteralPath $Downloaded["nsis_tauri_utils.dll"] `
        -Destination (Join-Path $UnicodePlugins "nsis_tauri_utils.dll")
    Assert-RequiredNsisLayout $PreparedNsis

    New-Item -ItemType Directory -Path $ResolvedToolCacheRoot -Force | Out-Null
    $FinalNsis = Join-Path $ResolvedToolCacheRoot "NSIS"
    Remove-AirWikiWindowsStagingPath `
        -Path $FinalNsis `
        -AllowedRoot $ResolvedToolCacheRoot `
        -Label "pinned NSIS toolchain destination"
    Move-Item -LiteralPath $PreparedNsis -Destination $FinalNsis
    Assert-RequiredNsisLayout $FinalNsis
    Write-Output "Pinned NSIS toolchain verified and prepared."
} finally {
    Remove-AirWikiWindowsStagingPath `
        -Path $Scratch `
        -AllowedRoot ([IO.Path]::GetTempPath()) `
        -Label "pinned NSIS download staging"
    Remove-AirWikiWindowsStagingPath `
        -Path $Stage `
        -AllowedRoot $ToolCacheParent `
        -Label "pinned NSIS toolchain staging"
}
