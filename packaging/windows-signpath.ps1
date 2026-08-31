$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$script:SignPathCodeSigningEku = "1.3.6.1.5.5.7.3.3"
$script:SignPathTimestampingEku = "1.3.6.1.5.5.7.3.8"

function Get-SignPathCertificateFingerprint(
    [System.Security.Cryptography.X509Certificates.X509Certificate2] $Certificate
) {
    $Hasher = [Security.Cryptography.SHA256]::Create()
    try {
        return [BitConverter]::ToString($Hasher.ComputeHash($Certificate.RawData)).Replace("-", "")
    } finally {
        $Hasher.Dispose()
    }
}

function Get-VerifiedWindowsSignTool {
    $SupportedVersion = $env:AIRWIKI_WINDOWS_SDK_SIGNTOOL_VERSION
    if ([string]::IsNullOrWhiteSpace($SupportedVersion) -or
        $SupportedVersion -notmatch '^\d+\.\d+\.\d+\.\d+$') {
        throw "AIRWIKI_WINDOWS_SDK_SIGNTOOL_VERSION must name one supported Windows SDK version"
    }
    $SdkRoot = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\bin"
    $ExpectedPath = Join-Path $SdkRoot "$SupportedVersion\x64\signtool.exe"
    $Verified = Get-VerifiedWindowsRegularFile $ExpectedPath "pinned Windows SDK signtool"
    $Signature = Get-AuthenticodeSignature -LiteralPath $Verified
    if ($Signature.Status -ne [System.Management.Automation.SignatureStatus]::Valid -or
        $null -eq $Signature.SignerCertificate) {
        throw "Windows SDK signtool must have a valid native signature"
    }
    $Publisher = $Signature.SignerCertificate.GetNameInfo(
        [System.Security.Cryptography.X509Certificates.X509NameType]::SimpleName,
        $false
    )
    if ($Publisher -cne "Microsoft Corporation") {
        throw "pinned Windows SDK signtool publisher must be Microsoft Corporation"
    }
    $FileVersion = (Get-Item -LiteralPath $Verified).VersionInfo.FileVersion
    if ([string]::IsNullOrWhiteSpace($FileVersion) -or
        -not $FileVersion.StartsWith($SupportedVersion, [StringComparison]::Ordinal)) {
        throw "pinned Windows SDK signtool file version does not match $SupportedVersion"
    }
    return $Verified
}

function Assert-SignPathCertificateEku(
    [System.Security.Cryptography.X509Certificates.X509Certificate2] $Certificate,
    [string] $RequiredEku,
    [string] $Label
) {
    $EkuExtensions = @($Certificate.Extensions | Where-Object { $_.Oid.Value -eq "2.5.29.37" })
    if ($EkuExtensions.Count -ne 1) {
        throw "$Label certificate must contain exactly one enhanced-key-usage extension"
    }
    $Decoded = if ($EkuExtensions[0] -is [Security.Cryptography.X509Certificates.X509EnhancedKeyUsageExtension]) {
        $EkuExtensions[0]
    } else {
        [Security.Cryptography.X509Certificates.X509EnhancedKeyUsageExtension]::new(
            $EkuExtensions[0],
            $EkuExtensions[0].Critical
        )
    }
    $Matches = @($Decoded.EnhancedKeyUsages | Where-Object { $_.Value -eq $RequiredEku })
    if ($Matches.Count -ne 1) {
        throw "$Label certificate lacks the required code-signing usage"
    }
}

function Get-VerifiedSignPathSignature([string] $Path, [string] $Label) {
    $Verified = Get-VerifiedWindowsRegularFile $Path $Label
    $SignTool = Get-VerifiedWindowsSignTool
    & $SignTool verify /pa /all /tw $Verified | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "$Label failed independent Windows Authenticode policy verification"
    }
    # signtool's indexed verification rejects a second primary signature. This
    # prevents an appended signer from being silently accepted by `/all`.
    & $SignTool verify /pa /tw /ds 0 $Verified | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "$Label has no independently verifiable primary Authenticode signature"
    }
    & $SignTool verify /pa /tw /ds 1 $Verified | Out-Null
    if ($LASTEXITCODE -eq 0) {
        throw "$Label must not contain more than one primary Authenticode signature"
    }
    $Signature = Get-AuthenticodeSignature -LiteralPath $Verified
    if ($Signature.Status -ne [System.Management.Automation.SignatureStatus]::Valid -or
        $null -eq $Signature.SignerCertificate -or
        $null -eq $Signature.TimeStamperCertificate) {
        throw "$Label must have one valid, timestamped Authenticode signature"
    }
    Assert-SignPathCertificateEku `
        $Signature.SignerCertificate `
        $script:SignPathCodeSigningEku `
        "$Label signer"
    Assert-SignPathCertificateEku `
        $Signature.TimeStamperCertificate `
        $script:SignPathTimestampingEku `
        "$Label timestamp"
    return [PSCustomObject]@{
        Path = $Verified
        Fingerprint = Get-SignPathCertificateFingerprint $Signature.SignerCertificate
        Subject = $Signature.SignerCertificate.Subject
    }
}

function Assert-SameSignPathSigner([object] $Expected, [object] $Actual, [string] $Label) {
    if (-not [String]::Equals(
        [string] $Expected.Fingerprint,
        [string] $Actual.Fingerprint,
        [StringComparison]::Ordinal
    ) -or -not [String]::Equals(
        [string] $Expected.Subject,
        [string] $Actual.Subject,
        [StringComparison]::Ordinal
    )) {
        throw "$Label signer does not match the AirWiki desktop signer"
    }
}

function Assert-ExpectedSignPathSigner([object] $Signer) {
    $Configured = $env:AIRWIKI_WINDOWS_SIGNER_SHA256
    if ([string]::IsNullOrWhiteSpace($Configured)) {
        throw "AIRWIKI_WINDOWS_SIGNER_SHA256 must contain an uppercase SHA-256 certificate fingerprint"
    }
    $Expected = @($Configured.Split(',') | ForEach-Object { $_.Trim() })
    if ($Expected.Count -lt 1 -or $Expected.Count -gt 2 -or
        @($Expected | Where-Object { $_ -cnotmatch '^[0-9A-F]{64}$' }).Count -ne 0 -or
        @($Expected | Select-Object -Unique).Count -ne $Expected.Count) {
        throw "AIRWIKI_WINDOWS_SIGNER_SHA256 must contain one or two distinct uppercase SHA-256 certificate fingerprints"
    }
    if (-not ($Expected -ccontains [string] $Signer.Fingerprint)) {
        throw "signed artifact does not match the configured AirWiki certificate"
    }
}
