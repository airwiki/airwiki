param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string] $Path
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
. (Join-Path $PSScriptRoot "windows-signing.ps1")

function Test-PathBelowRoot([string] $Candidate, [string] $AllowedRoot) {
    $Separators = [char[]]@([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
    $Prefix = $AllowedRoot.TrimEnd($Separators) + [IO.Path]::DirectorySeparatorChar
    return $Candidate.StartsWith($Prefix, [StringComparison]::OrdinalIgnoreCase)
}

function Save-SignedUninstallerReceipt([string] $SignedPath) {
    $SigningTemp = [IO.Path]::GetFullPath((Join-Path $Root "target\packages\windows-signing-temp"))
    $Separators = [char[]]@([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
    $SigningTempPrefix = $SigningTemp.TrimEnd($Separators) + [IO.Path]::DirectorySeparatorChar
    if (-not $SignedPath.StartsWith($SigningTempPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        return
    }
    $ReceiptDirectory = Join-Path $Root "target\windows-uninstaller"
    $Receipt = Join-Path $ReceiptDirectory "airwiki-uninstall.exe"
    $Partial = "$Receipt.partial"
    New-Item -ItemType Directory -Path $ReceiptDirectory -Force | Out-Null
    Assert-NoReparsePath $ReceiptDirectory
    if ((Test-Path -LiteralPath $Receipt) -or (Test-Path -LiteralPath $Partial)) {
        throw "NSIS attempted to materialize more than one signed uninstaller"
    }
    Copy-Item -LiteralPath $SignedPath -Destination $Partial
    Assert-NoReparsePath $Partial
    Move-Item -LiteralPath $Partial -Destination $Receipt
}

$InputItem = Get-Item -LiteralPath $Path -Force
Assert-NoReparsePath $InputItem.FullName
$ResolvedPath = $InputItem.FullName
$PackageRoot = (Resolve-Path -LiteralPath (Join-Path $Root "target\packages\windows")).Path
$SigningTempRoot = (Resolve-Path -LiteralPath (Join-Path $Root "target\packages\windows-signing-temp")).Path
$FinalInstaller = [IO.Path]::GetFullPath(
    (Join-Path $PackageRoot "airwiki_0.2.0_x64-setup.exe")
)
$PreSignedMainBinary = [IO.Path]::GetFullPath(
    (Join-Path $Root "target\x86_64-pc-windows-msvc\release\airwiki.exe")
)
Assert-NoReparsePath $PackageRoot
Assert-NoReparsePath $SigningTempRoot
$InPackageRoot = Test-PathBelowRoot $ResolvedPath $PackageRoot
$InSigningTempRoot = Test-PathBelowRoot $ResolvedPath $SigningTempRoot
$IsFinalInstaller = $ResolvedPath.Equals(
    $FinalInstaller,
    [StringComparison]::OrdinalIgnoreCase
)
$IsPreSignedMainBinary = $ResolvedPath.Equals(
    $PreSignedMainBinary,
    [StringComparison]::OrdinalIgnoreCase
)
$Extension = [IO.Path]::GetExtension($ResolvedPath)
if (($InPackageRoot -and (-not $IsFinalInstaller -or $Extension -ne ".exe")) -or
    ($InSigningTempRoot -and $Extension -ne ".exe" -and $Extension -ne ".tmp") -or
    (-not $InPackageRoot -and -not $InSigningTempRoot -and -not $IsPreSignedMainBinary)) {
    throw "signing target must be the pre-signed desktop, the exact final installer or the NSIS uninstaller temporary executable"
}

if ($IsPreSignedMainBinary) {
    $ExpectedMachineLow = 0x64
    $ExpectedMachineHigh = 0x86
    $ExpectedMachineLabel = "Windows x64 desktop"
} else {
    # cargo-packager 0.11.8 pins NSIS 3.09, whose Unicode installer and
    # uninstaller stubs are PE32 I386 even when they carry an x64 payload.
    $ExpectedMachineLow = 0x4c
    $ExpectedMachineHigh = 0x01
    $ExpectedMachineLabel = "NSIS 3.09 I386 executable"
}

$Stream = [IO.File]::OpenRead($ResolvedPath)
try {
    $Header = [byte[]]::new(64)
    if ($Stream.Read($Header, 0, $Header.Length) -ne $Header.Length -or
        $Header[0] -ne 0x4d -or $Header[1] -ne 0x5a) {
        throw "signing target is not a PE executable"
    }
    $PeOffset = [BitConverter]::ToUInt32($Header, 0x3c)
    if ($PeOffset -gt ($Stream.Length - 6)) {
        throw "signing target has a truncated PE header"
    }
    $null = $Stream.Seek($PeOffset, [IO.SeekOrigin]::Begin)
    $PeHeader = [byte[]]::new(6)
    if ($Stream.Read($PeHeader, 0, $PeHeader.Length) -ne $PeHeader.Length -or
        $PeHeader[0] -ne 0x50 -or $PeHeader[1] -ne 0x45 -or
        $PeHeader[2] -ne 0 -or $PeHeader[3] -ne 0 -or
        $PeHeader[4] -ne $ExpectedMachineLow -or
        $PeHeader[5] -ne $ExpectedMachineHigh) {
        throw "signing target is not the expected $ExpectedMachineLabel"
    }
} finally {
    $Stream.Dispose()
}

$Existing = Get-AuthenticodeSignature -LiteralPath $ResolvedPath
if ($Existing.Status -eq [System.Management.Automation.SignatureStatus]::Valid) {
    $null = Assert-ExpectedWindowsSigner $ResolvedPath
    Save-SignedUninstallerReceipt $ResolvedPath
    exit 0
}
if ($IsPreSignedMainBinary) {
    throw "cargo-packager main executable must already have the expected Authenticode signature"
}
if ($Existing.Status -ne [System.Management.Automation.SignatureStatus]::NotSigned) {
    throw "refusing to replace a non-valid existing Authenticode signature: $($Existing.Status)"
}

$Endpoint = $env:AIRWIKI_ARTIFACT_SIGNING_ENDPOINT
$Account = $env:AIRWIKI_ARTIFACT_SIGNING_ACCOUNT
$Profile = $env:AIRWIKI_ARTIFACT_SIGNING_PROFILE
if ([string]::IsNullOrWhiteSpace($Endpoint) -or
    [string]::IsNullOrWhiteSpace($Account) -or
    [string]::IsNullOrWhiteSpace($Profile)) {
    throw "Artifact Signing endpoint, account and profile are required"
}
if ($Endpoint -notmatch '^https://[a-z0-9.-]+\.codesigning\.azure\.net/?$') {
    throw "Artifact Signing endpoint is not an approved HTTPS service endpoint"
}
$PinnedArtifactSigningVersion = [Version] "0.1.8"
Import-Module ArtifactSigning `
    -RequiredVersion $PinnedArtifactSigningVersion `
    -Force `
    -ErrorAction Stop
$ArtifactSigningCommand = Get-Command Invoke-ArtifactSigning -ErrorAction Stop
if (-not [String]::Equals(
        $ArtifactSigningCommand.ModuleName,
        "ArtifactSigning",
        [StringComparison]::Ordinal
    ) -or
    $null -eq $ArtifactSigningCommand.Module -or
    $ArtifactSigningCommand.Module.Version -ne $PinnedArtifactSigningVersion) {
    throw "Invoke-ArtifactSigning must come from ArtifactSigning module version 0.1.8"
}

& $ArtifactSigningCommand `
    -Endpoint $Endpoint `
    -CodeSigningAccountName $Account `
    -CertificateProfileName $Profile `
    -Files $ResolvedPath `
    -FileDigest "SHA256" `
    -TimestampRfc3161 "http://timestamp.acs.microsoft.com" `
    -TimestampDigest "SHA256" `
    -ExcludeWorkloadIdentityCredential $true `
    -ExcludeAzureCliCredential $false

$null = Assert-ExpectedWindowsSigner $ResolvedPath
Save-SignedUninstallerReceipt $ResolvedPath
