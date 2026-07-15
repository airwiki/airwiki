[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $ToolRoot,

    [string] $ArtifactDirectory,

    [switch] $ValidateOnly
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

. (Join-Path $PSScriptRoot "windows-safe-staging.ps1")

$PinnedMsiName = "7z2602-x64.msi"
$PinnedMsiUrl = "https://github.com/ip7z/7zip/releases/download/26.02/7z2602-x64.msi"
$PinnedMsiLength = 1999872
$PinnedMsiSha256 = "db407a4f6d4999e5c7bc00ce8a882be94717b56e7fa68140fe3f12605d91643e"
$PinnedFiles = @(
    @{
        Name = "7z.exe"
        Length = 576000
        Sha256 = "83967f1b02b43c4efeda302795722c809e0e81b8307de73558d10484d5676a7d"
        PeMachine = 0x8664
    },
    @{
        Name = "7z.dll"
        Length = 1906688
        Sha256 = "69fd4df057985c40e510e2fac182881c7f85e90aa13ec703f763a8fdb2ce61f8"
        PeMachine = 0x8664
    },
    @{
        Name = "License.txt"
        Length = 6031
        Sha256 = "519ac0a4bded9c18ea02e0afb71f663d8c47373bd9facd3ac96a79f51d77765d"
    }
)

function Assert-RegularFile([string] $Path, [string] $Label) {
    $Item = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
    if ($Item.PSIsContainer -or ($Item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "$Label must be a regular file"
    }
    return $Item
}

function Assert-ExpectedFile([string] $Path, [hashtable] $Expected) {
    $Item = Assert-RegularFile $Path "Pinned 7-Zip file"
    if ($Item.Length -ne $Expected.Length) {
        throw "Unexpected length for $($Expected.Name)"
    }
    $ActualHash = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
    if (-not $ActualHash.Equals($Expected.Sha256, [StringComparison]::OrdinalIgnoreCase)) {
        throw "SHA-256 mismatch for $($Expected.Name)"
    }
}

function Assert-X64Pe([string] $Path, [string] $Label) {
    $Bytes = [IO.File]::ReadAllBytes($Path)
    if ($Bytes.Length -lt 64 -or $Bytes[0] -ne 0x4d -or $Bytes[1] -ne 0x5a) {
        throw "$Label is not a PE file"
    }
    $Offset = [BitConverter]::ToUInt32($Bytes, 0x3c)
    if ($Offset + 6 -gt $Bytes.Length -or
        $Bytes[$Offset] -ne 0x50 -or
        $Bytes[$Offset + 1] -ne 0x45 -or
        $Bytes[$Offset + 2] -ne 0 -or
        $Bytes[$Offset + 3] -ne 0) {
        throw "$Label has an invalid PE header"
    }
    $Machine = [BitConverter]::ToUInt16($Bytes, $Offset + 4)
    if ($Machine -ne 0x8664) {
        throw "$Label is not Windows x64"
    }
}

function Assert-Prepared7ZipLayout([string] $PreparedRoot) {
    if (-not (Test-Path -LiteralPath $PreparedRoot -PathType Container)) {
        throw "Pinned 7-Zip tool root is missing"
    }
    $RootItem = Get-Item -LiteralPath $PreparedRoot -Force
    if ($RootItem.Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw "Pinned 7-Zip tool root must not be a reparse point"
    }

    $Entries = @(Get-ChildItem -LiteralPath $PreparedRoot -Force)
    if ($Entries.Count -ne $PinnedFiles.Count) {
        throw "Pinned 7-Zip tool root has an unexpected layout"
    }
    foreach ($Entry in $Entries) {
        if ($Entry.PSIsContainer -or ($Entry.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw "Pinned 7-Zip tool root contains a non-regular entry"
        }
    }

    foreach ($Expected in $PinnedFiles) {
        $Path = Join-Path $PreparedRoot $Expected.Name
        Assert-ExpectedFile $Path $Expected
        if ($Expected.ContainsKey("PeMachine")) {
            Assert-X64Pe $Path "Pinned $($Expected.Name)"
        }
    }
}

function Get-PinnedMsi([string] $Destination, [string] $LocalArtifactDirectory) {
    if ([string]::IsNullOrWhiteSpace($LocalArtifactDirectory)) {
        Invoke-WebRequest -UseBasicParsing -Uri $PinnedMsiUrl -OutFile $Destination
    } else {
        $Source = Join-Path $LocalArtifactDirectory $PinnedMsiName
        Assert-RegularFile $Source "Pinned 7-Zip MSI" | Out-Null
        Copy-Item -LiteralPath $Source -Destination $Destination
    }

    $Item = Assert-RegularFile $Destination "Downloaded 7-Zip MSI"
    if ($Item.Length -ne $PinnedMsiLength) {
        throw "Unexpected length for $PinnedMsiName"
    }
    $ActualHash = (Get-FileHash -LiteralPath $Destination -Algorithm SHA256).Hash
    if (-not $ActualHash.Equals($PinnedMsiSha256, [StringComparison]::OrdinalIgnoreCase)) {
        throw "SHA-256 mismatch for $PinnedMsiName"
    }
}

$ResolvedToolRoot = [IO.Path]::GetFullPath($ToolRoot)
if ($ValidateOnly) {
    Assert-Prepared7ZipLayout $ResolvedToolRoot
    Write-Output $ResolvedToolRoot
    return
}

if (Test-Path -LiteralPath $ResolvedToolRoot) {
    Assert-Prepared7ZipLayout $ResolvedToolRoot
    Write-Output $ResolvedToolRoot
    return
}

$ResolvedArtifactDirectory = if ([string]::IsNullOrWhiteSpace($ArtifactDirectory)) {
    ""
} else {
    (Resolve-Path -LiteralPath $ArtifactDirectory -ErrorAction Stop).Path
}
$ToolParent = Split-Path -Parent $ResolvedToolRoot
if ([string]::IsNullOrWhiteSpace($ToolParent)) {
    throw "Pinned 7-Zip tool root must have a parent directory"
}
New-Item -ItemType Directory -Path $ToolParent -Force | Out-Null

$Scratch = Join-Path ([IO.Path]::GetTempPath()) "airwiki-7zip-$([Guid]::NewGuid().ToString('N'))"
$Stage = Join-Path $ToolParent ".airwiki-7zip-stage-$([Guid]::NewGuid().ToString('N'))"

try {
    New-Item -ItemType Directory -Path $Scratch, $Stage -Force | Out-Null
    $Msi = Join-Path $Scratch $PinnedMsiName
    Get-PinnedMsi $Msi $ResolvedArtifactDirectory

    $AdministrativeImage = Join-Path $Scratch "administrative-image"
    New-Item -ItemType Directory -Path $AdministrativeImage | Out-Null
    $MsiExec = Join-Path $env:SystemRoot "System32\msiexec.exe"
    Assert-RegularFile $MsiExec "Windows Installer" | Out-Null
    $MsiArguments = "/a `"$Msi`" /qn /norestart TARGETDIR=`"$AdministrativeImage`""
    $Process = Start-Process `
        -FilePath $MsiExec `
        -ArgumentList $MsiArguments `
        -NoNewWindow `
        -Wait `
        -PassThru
    if ($Process.ExitCode -ne 0) {
        throw "Pinned 7-Zip administrative extraction failed with exit code $($Process.ExitCode)"
    }

    $ExtractedRoot = Join-Path $AdministrativeImage "Files\7-Zip"
    foreach ($Expected in $PinnedFiles) {
        $Source = Join-Path $ExtractedRoot $Expected.Name
        Assert-ExpectedFile $Source $Expected
        Copy-Item -LiteralPath $Source -Destination (Join-Path $Stage $Expected.Name)
    }
    Assert-Prepared7ZipLayout $Stage

    # The staging directory lives beside the destination, so the final rename
    # never exposes a partially copied extractor.
    Move-Item -LiteralPath $Stage -Destination $ResolvedToolRoot
    Assert-Prepared7ZipLayout $ResolvedToolRoot
    Write-Output $ResolvedToolRoot
} finally {
    Remove-AirWikiWindowsStagingPath `
        -Path $Scratch `
        -AllowedRoot ([IO.Path]::GetTempPath()) `
        -Label "pinned 7-Zip extraction staging"
    Remove-AirWikiWindowsStagingPath `
        -Path $Stage `
        -AllowedRoot $ToolParent `
        -Label "pinned 7-Zip tool staging"
}
