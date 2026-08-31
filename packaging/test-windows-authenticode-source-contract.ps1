$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    throw "Windows Authenticode source-contract tests require Windows"
}

$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$Policy = Join-Path $PSScriptRoot "windows-authenticode.ps1"
$Source = Get-Content -LiteralPath $Policy -Raw
$SigningWorkflow = Get-Content -LiteralPath (Join-Path $Root ".github\workflows\windows-esigner.yml") -Raw

function Assert-Throws([scriptblock] $Action, [string] $Label) {
    try {
        & $Action
    } catch {
        return
    }
    throw "$Label must fail closed"
}

foreach ($Required in @(
    "AIRWIKI_WINDOWS_SDK_SIGNTOOL_VERSION",
    "pinned Windows SDK signtool",
    "SignatureStatus]::Valid",
    "Microsoft Corporation",
    "TimeStamperCertificate",
    "exactly one configured AirWiki code-signing certificate must be present"
)) {
    if ($Source.IndexOf($Required, [StringComparison]::Ordinal) -lt 0) {
        throw "Windows Authenticode source contract is missing required guard: $Required"
    }
}

foreach ($Required in @(
    "AIRWIKI_ESIGNER_SECRET_TRANSPORT_APPROVED",
    "sslcom-esigner-secret-transport-v1",
    "eSigner secret transport is not approved for this protected environment"
)) {
    if ($SigningWorkflow.IndexOf($Required, [StringComparison]::Ordinal) -lt 0) {
        throw "Windows eSigner workflow is missing required secret-transport gate: $Required"
    }
}
$ExpectedProtectedEnvironment = @'
environment: windows-signing
    env:
      AIRWIKI_UPDATER_PUBLIC_KEY: ${{ vars.AIRWIKI_UPDATER_PUBLIC_KEY }}
      AIRWIKI_WINDOWS_SIGNER_SHA256: ${{ vars.AIRWIKI_WINDOWS_SIGNER_SHA256 }}
      AIRWIKI_ESIGNER_SECRET_TRANSPORT_APPROVED: ${{ vars.AIRWIKI_ESIGNER_SECRET_TRANSPORT_APPROVED }}
'@
$ExpectedProtectedEnvironment = $ExpectedProtectedEnvironment.TrimEnd()
if ($SigningWorkflow.IndexOf($ExpectedProtectedEnvironment, [StringComparison]::Ordinal) -lt 0) {
    throw "Windows eSigner secret-transport gate must be scoped to the protected signing job"
}

. $Policy
$OriginalVersion = [Environment]::GetEnvironmentVariable(
    "AIRWIKI_WINDOWS_SDK_SIGNTOOL_VERSION",
    "Process"
)
$OriginalFingerprint = [Environment]::GetEnvironmentVariable(
    "AIRWIKI_WINDOWS_SIGNER_SHA256",
    "Process"
)
try {
    [Environment]::SetEnvironmentVariable(
        "AIRWIKI_WINDOWS_SDK_SIGNTOOL_VERSION",
        $null,
        "Process"
    )
    Assert-Throws { Get-VerifiedWindowsSignTool } "missing Windows SDK version"

    [Environment]::SetEnvironmentVariable(
        "AIRWIKI_WINDOWS_SDK_SIGNTOOL_VERSION",
        "not-a-version",
        "Process"
    )
    Assert-Throws { Get-VerifiedWindowsSignTool } "malformed Windows SDK version"

    [Environment]::SetEnvironmentVariable(
        "AIRWIKI_WINDOWS_SIGNER_SHA256",
        "not-a-fingerprint",
        "Process"
    )
    Assert-Throws { Get-ConfiguredWindowsSignerFingerprints } "malformed signer fingerprint"

    $Duplicate = ("A" * 64) + "," + ("A" * 64)
    [Environment]::SetEnvironmentVariable(
        "AIRWIKI_WINDOWS_SIGNER_SHA256",
        $Duplicate,
        "Process"
    )
    Assert-Throws { Get-ConfiguredWindowsSignerFingerprints } "duplicate signer fingerprint"
} finally {
    [Environment]::SetEnvironmentVariable(
        "AIRWIKI_WINDOWS_SDK_SIGNTOOL_VERSION",
        $OriginalVersion,
        "Process"
    )
    [Environment]::SetEnvironmentVariable(
        "AIRWIKI_WINDOWS_SIGNER_SHA256",
        $OriginalFingerprint,
        "Process"
    )
}

Write-Host "Validated Windows Authenticode source contract and fail-closed local inputs"
