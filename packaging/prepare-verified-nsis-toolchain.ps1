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
        Name = "nsis-3.11.zip"
        Url = "https://github.com/tauri-apps/binary-releases/releases/download/nsis-3.11/nsis-3.11.zip"
        Sha256 = "c7d27f780ddb6cffb4730138cd1591e841f4b7edb155856901cdf5f214394fa1"
    },
    @{
        Name = "nsis_tauri_utils.dll"
        Url = "https://github.com/tauri-apps/nsis-tauri-utils/releases/download/nsis_tauri_utils-v0.5.3/nsis_tauri_utils.dll"
        Sha256 = "5ba143b5db4a87d32d6e7802e033330aae56cbceabe0d1e3ba41948385ad4709"
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
        "Plugins\x86-unicode\additional\nsis_tauri_utils.dll",
        "Include\MUI2.nsh",
        "Include\FileFunc.nsh",
        "Include\x64.nsh",
        "Include\nsDialogs.nsh",
        "Include\WinMessages.nsh",
        "Include\Win\COM.nsh",
        "Include\Win\Propkey.nsh",
        "Include\Win\RestartManager.nsh"
    )
    foreach ($RelativePath in $RequiredFiles) {
        Assert-RegularFile (Join-Path $NsisRoot $RelativePath) "Prepared NSIS toolchain entry"
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
    Expand-Archive -LiteralPath $Downloaded["nsis-3.11.zip"] -DestinationPath $NsisExtract

    $ExtractedNsis = Join-Path $NsisExtract "nsis-3.11"
    if (-not (Test-Path -LiteralPath $ExtractedNsis -PathType Container)) {
        throw "Pinned NSIS archive has an unexpected layout"
    }
    $PreparedNsis = Join-Path $Stage "NSIS"
    Move-Item -LiteralPath $ExtractedNsis -Destination $PreparedNsis
    $UnicodePlugins = Join-Path $PreparedNsis "Plugins\x86-unicode\additional"
    New-Item -ItemType Directory -Path $UnicodePlugins -Force | Out-Null
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
