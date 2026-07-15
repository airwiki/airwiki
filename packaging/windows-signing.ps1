$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$script:CodeSigningEku = "1.3.6.1.5.5.7.3.3"
$script:TimestampingEku = "1.3.6.1.5.5.7.3.8"
$script:ArtifactSigningEkuPrefix = "1.3.6.1.4.1.311.97."
$script:ArtifactSigningPublicTrustMarker = "1.3.6.1.4.1.311.97.1.0"
$script:Sha256Oid = "2.16.840.1.101.3.4.2.1"
$script:SpcIndirectDataOid = "1.3.6.1.4.1.311.2.1.4"
$script:NestedAuthenticodeSignatureOid = "1.3.6.1.4.1.311.2.4.1"
$script:Rfc3161TimestampAttributeOid = "1.2.840.113549.1.9.16.2.14"
$script:Rfc3161TimestampInfoOid = "1.2.840.113549.1.9.16.1.4"

function Assert-NoReparsePath([string] $Path) {
    if (-not (Test-Path -LiteralPath $Path)) {
        throw "path does not exist: $Path"
    }

    $Current = Get-Item -LiteralPath $Path -Force
    while ($null -ne $Current) {
        if (($Current.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "release path must not contain a symlink or reparse point: $($Current.FullName)"
        }

        $ParentPath = Split-Path -Parent $Current.FullName
        if ([string]::IsNullOrWhiteSpace($ParentPath) -or
            [String]::Equals($ParentPath, $Current.FullName, [StringComparison]::OrdinalIgnoreCase)) {
            break
        }
        $Current = Get-Item -LiteralPath $ParentPath -Force
    }
}

function Get-CertificateEkuOids(
    [System.Security.Cryptography.X509Certificates.X509Certificate2] $Certificate
) {
    $Extension = @($Certificate.Extensions | Where-Object {
        $_.Oid.Value -eq "2.5.29.37"
    })
    if ($Extension.Count -ne 1) {
        throw "signing certificate must contain exactly one enhanced-key-usage extension"
    }
    if ($Extension[0] -is [System.Security.Cryptography.X509Certificates.X509EnhancedKeyUsageExtension]) {
        $Decoded = $Extension[0]
    } else {
        $Decoded = [System.Security.Cryptography.X509Certificates.X509EnhancedKeyUsageExtension]::new(
            $Extension[0],
            $Extension[0].Critical
        )
    }
    return @($Decoded.EnhancedKeyUsages | ForEach-Object { $_.Value })
}

function Assert-SingleRequiredEku(
    [string[]] $Ekus,
    [string] $Required,
    [string] $Label
) {
    $Matches = @($Ekus | Where-Object {
        [String]::Equals($_, $Required, [StringComparison]::Ordinal)
    })
    if ($Matches.Count -ne 1) {
        throw "$Label must contain exactly one $Required enhanced key usage"
    }
}

function Get-SingleArtifactSigningDurableIdentity(
    [string[]] $SignerEkus,
    [string] $Path
) {
    $DurableIdentities = @($SignerEkus | Where-Object {
        $_.StartsWith($script:ArtifactSigningEkuPrefix, [StringComparison]::Ordinal) -and
        $_ -ne $script:ArtifactSigningPublicTrustMarker
    })
    if ($DurableIdentities.Count -ne 1) {
        throw "signer certificate must contain exactly one Artifact Signing subscriber identity: $Path"
    }
    return $DurableIdentities[0]
}

function Assert-ValidExpectedWindowsSignerIdentity([string] $Expected) {
    if ([string]::IsNullOrWhiteSpace($Expected)) {
        throw "AIRWIKI_WINDOWS_SIGNER_IDENTITY is required for a signed release"
    }
    if (-not [String]::Equals($Expected, $Expected.Trim(), [StringComparison]::Ordinal)) {
        throw "AIRWIKI_WINDOWS_SIGNER_IDENTITY must not contain surrounding whitespace"
    }
    $Pattern = '^artifact-signing-eku:1\.3\.6\.1\.4\.1\.311\.97\.(?:0|[1-9][0-9]*)(?:\.(?:0|[1-9][0-9]*))*$'
    if ($Expected -cnotmatch $Pattern -or
        $Expected -ceq "artifact-signing-eku:$($script:ArtifactSigningPublicTrustMarker)") {
        throw "AIRWIKI_WINDOWS_SIGNER_IDENTITY must be one exact Artifact Signing subscriber EKU"
    }
}

function Get-SingleDerObject([byte[]] $Bytes, [string] $Path) {
    if ($Bytes.Length -lt 2 -or $Bytes[0] -ne 0x30) {
        throw "embedded Authenticode content is not a DER sequence: $Path"
    }
    $LengthOctet = [int] $Bytes[1]
    if (($LengthOctet -band 0x80) -eq 0) {
        $HeaderLength = 2
        $ContentLength = [uint64] $LengthOctet
    } else {
        $LengthBytes = $LengthOctet -band 0x7f
        if ($LengthBytes -lt 1 -or $LengthBytes -gt 4 -or
            $Bytes.Length -lt (2 + $LengthBytes) -or $Bytes[2] -eq 0) {
            throw "embedded Authenticode content has an invalid DER length: $Path"
        }
        $HeaderLength = 2 + $LengthBytes
        $ContentLength = [uint64] 0
        for ($Index = 0; $Index -lt $LengthBytes; $Index++) {
            $ContentLength = ($ContentLength * 256) + $Bytes[2 + $Index]
        }
        if ($ContentLength -lt 128) {
            throw "embedded Authenticode content uses a non-minimal DER length: $Path"
        }
    }
    $EncodedLength = [uint64] $HeaderLength + $ContentLength
    if ($EncodedLength -gt [uint64] $Bytes.Length -or
        ([uint64] $Bytes.Length - $EncodedLength) -gt 7) {
        throw "embedded Authenticode content has an invalid padded length: $Path"
    }
    for ($Index = [int] $EncodedLength; $Index -lt $Bytes.Length; $Index++) {
        if ($Bytes[$Index] -ne 0) {
            throw "embedded Authenticode content has nonzero certificate padding: $Path"
        }
    }
    $Encoded = [byte[]]::new([int] $EncodedLength)
    [Array]::Copy($Bytes, 0, $Encoded, 0, $Encoded.Length)
    return ,$Encoded
}

function Get-OnlyEmbeddedAuthenticodeCms([string] $Path) {
    $Bytes = [IO.File]::ReadAllBytes($Path)
    if ($Bytes.Length -lt 64 -or $Bytes[0] -ne 0x4d -or $Bytes[1] -ne 0x5a) {
        throw "release executable is not a complete PE image: $Path"
    }
    $PeOffset = [BitConverter]::ToUInt32($Bytes, 0x3c)
    if ($PeOffset -gt ($Bytes.Length - 26) -or
        $Bytes[$PeOffset] -ne 0x50 -or $Bytes[$PeOffset + 1] -ne 0x45 -or
        $Bytes[$PeOffset + 2] -ne 0 -or $Bytes[$PeOffset + 3] -ne 0) {
        throw "release executable has an invalid PE header: $Path"
    }

    $OptionalHeader = $PeOffset + 24
    $Magic = [BitConverter]::ToUInt16($Bytes, $OptionalHeader)
    if ($Magic -eq 0x10b) {
        $DataDirectories = $OptionalHeader + 96
    } elseif ($Magic -eq 0x20b) {
        $DataDirectories = $OptionalHeader + 112
    } else {
        throw "release executable has an unsupported PE optional header: $Path"
    }
    $SecurityDirectory = $DataDirectories + (8 * 4)
    if ($SecurityDirectory -gt ($Bytes.Length - 8)) {
        throw "release executable has a truncated PE security directory: $Path"
    }
    $CertificateOffset = [BitConverter]::ToUInt32($Bytes, $SecurityDirectory)
    $CertificateSize = [BitConverter]::ToUInt32($Bytes, $SecurityDirectory + 4)
    $CertificateEnd = [uint64] $CertificateOffset + [uint64] $CertificateSize
    if ($CertificateOffset -eq 0 -or $CertificateSize -lt 8 -or
        $CertificateEnd -gt [uint64] $Bytes.Length) {
        throw "release executable has an invalid PE certificate table: $Path"
    }

    $Entries = @()
    $Position = [uint64] $CertificateOffset
    while ($Position -lt $CertificateEnd) {
        if ($Position + 8 -gt $CertificateEnd) {
            throw "release executable has a truncated WIN_CERTIFICATE entry: $Path"
        }
        $Offset = [int] $Position
        $Length = [BitConverter]::ToUInt32($Bytes, $Offset)
        $Revision = [BitConverter]::ToUInt16($Bytes, $Offset + 4)
        $CertificateType = [BitConverter]::ToUInt16($Bytes, $Offset + 6)
        if ($Length -lt 8 -or $Position + [uint64] $Length -gt $CertificateEnd -or
            $Revision -ne 0x0200 -or $CertificateType -ne 0x0002) {
            throw "release executable has an unsupported WIN_CERTIFICATE entry: $Path"
        }
        if ($Length -gt [int]::MaxValue) {
            throw "release executable has an oversized WIN_CERTIFICATE entry: $Path"
        }
        $ContentLength = [int] $Length - 8
        $Content = [byte[]]::new($ContentLength)
        [Array]::Copy($Bytes, $Offset + 8, $Content, 0, $ContentLength)
        $Entries += ,(Get-SingleDerObject $Content $Path)
        $AlignedLength = [uint64] $Length + 7
        $AlignedLength -= $AlignedLength % 8
        $Position += $AlignedLength
    }
    if ($Position -ne $CertificateEnd -or $Entries.Count -ne 1) {
        throw "release executable must contain exactly one embedded Authenticode certificate: $Path"
    }
    return ,$Entries[0]
}

function Get-AlgorithmIdentifierOid($Reader) {
    $Algorithm = $Reader.ReadSequence()
    $Oid = $Algorithm.ReadObjectIdentifier()
    if ($Algorithm.HasData) {
        $Algorithm.ReadNull()
    }
    if ($Algorithm.HasData) {
        throw "algorithm identifier contains unexpected trailing fields"
    }
    return $Oid
}

function Get-SpcIndirectDataDigestOid([byte[]] $Content) {
    $Memory = [System.ReadOnlyMemory[byte]]::new($Content)
    $Reader = [System.Formats.Asn1.AsnReader]::new(
        $Memory,
        [System.Formats.Asn1.AsnEncodingRules]::BER
    )
    $Sequence = $Reader.ReadSequence()
    $null = $Sequence.ReadEncodedValue()
    $DigestInfo = $Sequence.ReadSequence()
    $Oid = Get-AlgorithmIdentifierOid $DigestInfo
    $null = $DigestInfo.ReadOctetString()
    if ($DigestInfo.HasData -or $Sequence.HasData -or $Reader.HasData) {
        throw "SPC_INDIRECT_DATA_CONTENT contains unexpected trailing fields"
    }
    return $Oid
}

function Get-Rfc3161MessageImprintOid([byte[]] $Content) {
    $Memory = [System.ReadOnlyMemory[byte]]::new($Content)
    $Reader = [System.Formats.Asn1.AsnReader]::new(
        $Memory,
        [System.Formats.Asn1.AsnEncodingRules]::BER
    )
    $Sequence = $Reader.ReadSequence()
    $null = $Sequence.ReadInteger()
    $null = $Sequence.ReadObjectIdentifier()
    $MessageImprint = $Sequence.ReadSequence()
    $Oid = Get-AlgorithmIdentifierOid $MessageImprint
    $null = $MessageImprint.ReadOctetString()
    if ($MessageImprint.HasData) {
        throw "RFC3161 message imprint contains unexpected trailing fields"
    }
    return $Oid
}

function Assert-AuthenticodeSha256Policy([string] $Path) {
    try {
        Add-Type -AssemblyName System.Security.Cryptography.Pkcs -ErrorAction Stop
        Add-Type -AssemblyName System.Formats.Asn1 -ErrorAction Stop
    } catch {
        throw "PowerShell cannot load the cryptographic assemblies required for release verification"
    }

    $CmsBytes = Get-OnlyEmbeddedAuthenticodeCms $Path
    $Cms = [System.Security.Cryptography.Pkcs.SignedCms]::new()
    $Cms.Decode($CmsBytes)
    $Cms.CheckSignature($true)
    if ($Cms.ContentInfo.ContentType.Value -ne $script:SpcIndirectDataOid -or
        $Cms.SignerInfos.Count -ne 1) {
        throw "release executable must contain one SPC_INDIRECT_DATA_CONTENT signer: $Path"
    }
    $Signer = $Cms.SignerInfos[0]
    if ($Signer.DigestAlgorithm.Value -ne $script:Sha256Oid -or
        (Get-SpcIndirectDataDigestOid $Cms.ContentInfo.Content) -ne $script:Sha256Oid) {
        throw "release executable must use SHA-256 for its Authenticode signature and file digest: $Path"
    }
    $NestedSignatures = @($Signer.UnsignedAttributes | Where-Object {
        $_.Oid.Value -eq $script:NestedAuthenticodeSignatureOid
    })
    if ($NestedSignatures.Count -ne 0) {
        throw "release executable must not contain a nested Authenticode signature: $Path"
    }
    if ($Signer.CounterSignerInfos.Count -ne 0) {
        throw "release executable must not contain a legacy Authenticode timestamp: $Path"
    }
    $TimestampAttributes = @($Signer.UnsignedAttributes | Where-Object {
        $_.Oid.Value -eq $script:Rfc3161TimestampAttributeOid
    })
    if ($TimestampAttributes.Count -ne 1 -or $TimestampAttributes[0].Values.Count -ne 1) {
        throw "release executable must contain exactly one RFC3161 timestamp token: $Path"
    }
    $TimestampCms = [System.Security.Cryptography.Pkcs.SignedCms]::new()
    $TimestampCms.Decode($TimestampAttributes[0].Values[0].RawData)
    $TimestampCms.CheckSignature($true)
    if ($TimestampCms.ContentInfo.ContentType.Value -ne $script:Rfc3161TimestampInfoOid -or
        $TimestampCms.SignerInfos.Count -ne 1 -or
        $TimestampCms.SignerInfos[0].DigestAlgorithm.Value -ne $script:Sha256Oid -or
        (Get-Rfc3161MessageImprintOid $TimestampCms.ContentInfo.Content) -ne $script:Sha256Oid) {
        throw "release executable must use one SHA-256 RFC3161 timestamp token: $Path"
    }
}

function Get-ValidAuthenticodeIdentity([string] $Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "missing signed release executable: $Path"
    }
    Assert-NoReparsePath $Path
    $Item = Get-Item -LiteralPath $Path -Force
    $Signature = Get-AuthenticodeSignature -LiteralPath $Path
    if ($Signature.Status -ne [System.Management.Automation.SignatureStatus]::Valid -or
        $null -eq $Signature.SignerCertificate) {
        throw "Authenticode validation failed for $Path`: $($Signature.Status)"
    }
    if (-not [String]::Equals(
        [string] $Signature.SignatureType,
        "Authenticode",
        [StringComparison]::Ordinal
    )) {
        throw "release executable must contain an embedded Authenticode signature: $Path"
    }

    $SignToolCommands = @(Get-Command "signtool.exe" -CommandType Application -ErrorAction Stop)
    if ($SignToolCommands.Count -ne 1) {
        throw "exactly one SignTool executable must be available on PATH"
    }
    & $SignToolCommands[0].Source verify /pa /all /tw $Item.FullName *> $null
    if ($LASTEXITCODE -ne 0) {
        throw "SignTool validation failed for release executable: $Path"
    }
    & $SignToolCommands[0].Source verify /pa /tw /ds 0 $Item.FullName *> $null
    if ($LASTEXITCODE -ne 0) {
        throw "SignTool could not verify the primary release signature: $Path"
    }
    & $SignToolCommands[0].Source verify /pa /tw /ds 1 $Item.FullName *> $null
    if ($LASTEXITCODE -eq 0) {
        throw "release executable must contain exactly one Authenticode signature: $Path"
    }
    Assert-AuthenticodeSha256Policy $Path

    $SignerEkus = @(Get-CertificateEkuOids $Signature.SignerCertificate)
    Assert-SingleRequiredEku $SignerEkus $script:CodeSigningEku "signer certificate for $Path"
    Assert-SingleRequiredEku `
        $SignerEkus `
        $script:ArtifactSigningPublicTrustMarker `
        "signer certificate for $Path"
    if ($null -eq $Signature.TimeStamperCertificate) {
        throw "RFC3161 timestamp is missing from $Path"
    }
    $TimestampEkus = @(Get-CertificateEkuOids $Signature.TimeStamperCertificate)
    Assert-SingleRequiredEku `
        $TimestampEkus `
        $script:TimestampingEku `
        "timestamp certificate for $Path"

    $DurableIdentity = Get-SingleArtifactSigningDurableIdentity $SignerEkus $Path
    $ComparisonKey = "artifact-signing-eku:$DurableIdentity"

    return [PSCustomObject]@{
        ComparisonKey = $ComparisonKey
        Signature = $Signature
    }
}

function Assert-ExpectedWindowsSigner([string] $Path) {
    $Identity = Get-ValidAuthenticodeIdentity $Path
    $Expected = $env:AIRWIKI_WINDOWS_SIGNER_IDENTITY
    Assert-ValidExpectedWindowsSignerIdentity $Expected
    if (-not [String]::Equals($Identity.ComparisonKey, $Expected, [StringComparison]::Ordinal)) {
        throw "release executable signer identity does not match the protected release identity: $Path"
    }
    return $Identity
}

function Assert-SameWindowsSigner([object] $Expected, [object] $Actual, [string] $Label) {
    if (-not [String]::Equals(
        $Expected.ComparisonKey,
        $Actual.ComparisonKey,
        [StringComparison]::Ordinal
    )) {
        throw "$Label does not share the expected release signer identity"
    }
}
