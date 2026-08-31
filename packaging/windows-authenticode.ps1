$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$script:WindowsCodeSigningEku = "1.3.6.1.5.5.7.3.3"
$script:WindowsTimestampingEku = "1.3.6.1.5.5.7.3.8"

function Get-WindowsCertificateFingerprint(
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
    if (-not (Test-Path -LiteralPath $SdkRoot -PathType Container)) {
        throw "Windows SDK signing tools are missing"
    }
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

function Assert-WindowsCertificateEku(
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

function Get-VerifiedWindowsAuthenticodeSignature([string] $Path, [string] $Label) {
    $Verified = Get-VerifiedWindowsRegularFile $Path $Label
    $SignTool = Get-VerifiedWindowsSignTool
    & $SignTool verify /pa /all /tw $Verified | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "$Label failed independent Windows Authenticode policy verification"
    }
    $Signature = Get-AuthenticodeSignature -LiteralPath $Verified
    if ($Signature.Status -ne [System.Management.Automation.SignatureStatus]::Valid -or
        $null -eq $Signature.SignerCertificate -or
        $null -eq $Signature.TimeStamperCertificate) {
        throw "$Label must have one valid, timestamped Authenticode signature"
    }
    Assert-WindowsCertificateEku `
        $Signature.SignerCertificate `
        $script:WindowsCodeSigningEku `
        "$Label signer"
    Assert-WindowsCertificateEku `
        $Signature.TimeStamperCertificate `
        $script:WindowsTimestampingEku `
        "$Label timestamp"
    return [PSCustomObject]@{
        Path = $Verified
        Fingerprint = Get-WindowsCertificateFingerprint $Signature.SignerCertificate
        Subject = $Signature.SignerCertificate.Subject
    }
}

function Assert-SameWindowsSigner([object] $Expected, [object] $Actual, [string] $Label) {
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

function Assert-ExpectedWindowsSigner([object] $Signer) {
    $Expected = Get-ConfiguredWindowsSignerFingerprints
    if (-not ($Expected -ccontains [string] $Signer.Fingerprint)) {
        throw "signed artifact does not match the configured AirWiki certificate"
    }
}

function Get-ConfiguredWindowsSignerFingerprints {
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
    return $Expected
}

function Get-ConfiguredWindowsSigningCertificate {
    $Expected = Get-ConfiguredWindowsSignerFingerprints
    $Matches = @(Get-ChildItem Cert:\CurrentUser\My -CodeSigningCert | Where-Object {
        $Expected -ccontains (Get-WindowsCertificateFingerprint $_)
    })
    if ($Matches.Count -ne 1) {
        throw "exactly one configured AirWiki code-signing certificate must be present"
    }
    $Certificate = $Matches[0]
    $Now = [DateTime]::UtcNow
    if (-not $Certificate.HasPrivateKey -or
        $Certificate.NotBefore.ToUniversalTime() -gt $Now -or
        $Certificate.NotAfter.ToUniversalTime() -le $Now -or
        $Certificate.Thumbprint -cnotmatch '^[0-9A-F]{40}$') {
        throw "configured AirWiki code-signing certificate is not currently usable"
    }
    Assert-WindowsCertificateEku `
        $Certificate `
        $script:WindowsCodeSigningEku `
        "configured AirWiki signer"
    return $Certificate
}

function Invoke-WindowsAuthenticodeSigning([string] $SignTool, [string] $Path, [string] $Label) {
    $Artifact = Get-VerifiedWindowsRegularFile $Path $Label
    $Certificate = Get-ConfiguredWindowsSigningCertificate
    & $SignTool sign /fd SHA256 /tr http://ts.ssl.com /td SHA256 /sha1 $Certificate.Thumbprint $Artifact
    if ($LASTEXITCODE -ne 0) {
        throw "Authenticode signing failed for $Label"
    }
    $Signer = Get-VerifiedWindowsAuthenticodeSignature $Artifact $Label
    Assert-ExpectedWindowsSigner $Signer
}
