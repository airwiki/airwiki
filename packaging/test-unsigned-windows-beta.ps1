$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$TargetRoot = Join-Path $Root "target"
$TestRoot = Join-Path $TargetRoot "unsigned-beta-policy-test"
$PackageRoot = Join-Path $TestRoot "packages"
$OutputRoot = Join-Path $TestRoot "artifact"
. (Join-Path $PSScriptRoot "windows-safe-staging.ps1")
$Version = (& node.exe (Join-Path $PSScriptRoot "release-version.mjs")).Trim()
if ($LASTEXITCODE -ne 0) {
    throw "release version validation failed"
}

function Assert-Rejected([scriptblock] $Action, [string] $Label) {
    try {
        & $Action
        throw "$Label was unexpectedly accepted"
    } catch {
        if ($_.Exception.Message -eq "$Label was unexpectedly accepted") {
            throw
        }
    }
}

function Write-SyntheticMsi([string] $Path, [byte] $Marker) {
    $Bytes = [byte[]]::new(64)
    $Header = [byte[]] @(0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1)
    [Array]::Copy($Header, $Bytes, $Header.Length)
    $Bytes[$Bytes.Length - 1] = $Marker
    [IO.File]::WriteAllBytes($Path, $Bytes)
}

New-Item -ItemType Directory -Path $TargetRoot -Force | Out-Null
Remove-AirWikiWindowsStagingPath `
    -Path $TestRoot `
    -AllowedRoot $TargetRoot `
    -Label "unsigned beta policy test"
New-Item -ItemType Directory -Path $PackageRoot -Force | Out-Null

try {
    Write-SyntheticMsi (Join-Path $PackageRoot "AirWiki_${Version}_x64_en-US.msi") 1
    Write-SyntheticMsi (Join-Path $PackageRoot "AirWiki_${Version}_x64_es-ES.msi") 2
    & (Join-Path $PSScriptRoot "prepare-unsigned-windows-beta.ps1") `
        -PackageRoot $PackageRoot `
        -OutputRoot $OutputRoot `
        -Version $Version `
        -CommitSha ("a" * 40) `
        -Repository "airwiki/airwiki" `
        -WorkflowRunUrl "https://github.com/airwiki/airwiki/actions/runs/123" `
        -RetentionDays 30 `
        -GeneratedAtUtc ([DateTimeOffset]::Parse("2026-08-25T00:00:00Z"))

    $Entries = @(Get-ChildItem -LiteralPath $OutputRoot -File | Sort-Object -Property Name)
    if ($Entries.Count -ne 5) {
        throw "unsigned beta artifact must contain two installers and three metadata files"
    }
    $Provenance = Get-Content -LiteralPath (Join-Path $OutputRoot "PROVENANCE.json") -Raw |
        ConvertFrom-Json
    if ($Provenance.artifact_kind -cne "airwiki-windows-unsigned-beta" -or
        $Provenance.code_signing -cne "not-requested" -or
        $Provenance.supported_public_release -ne $false -or
        $Provenance.commit_sha -cne ("a" * 40) -or
        $Provenance.expires_at_utc -cne "2026-09-24T00:00:00Z" -or
        @($Provenance.installers).Count -ne 2) {
        throw "unsigned beta provenance does not match the closed schema"
    }

    $StrictUtf8 = [Text.UTF8Encoding]::new($false, $true)
    $Notice = [IO.File]::ReadAllText(
        (Join-Path $OutputRoot "UNSIGNED-BETA.txt"),
        $StrictUtf8
    )
    $SpanishTechnical = "t" + [char] 0x00e9 + "cnicas invitadas"
    $SpanishProtection = "PROTECCI" + [char] 0x00d3 + "N DE WINDOWS"
    if (-not $Notice.Contains($SpanishTechnical) -or
        -not $Notice.Contains($SpanishProtection) -or
        $Notice.Contains("{{")) {
        throw "unsigned beta notice must preserve UTF-8 and replace every value"
    }

    $ChecksumLines = @(Get-Content -LiteralPath (Join-Path $OutputRoot "SHA256SUMS.txt"))
    if ($ChecksumLines.Count -ne 2) {
        throw "unsigned beta checksum file must contain exactly two installers"
    }
    foreach ($Installer in @($Provenance.installers)) {
        $Path = Join-Path $OutputRoot $Installer.name
        $ActualHash = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($ActualHash -cne $Installer.sha256 -or
            $ChecksumLines -cnotcontains "$ActualHash  $($Installer.name)") {
            throw "unsigned beta installer checksum does not match its metadata"
        }
    }

    Assert-Rejected {
        & (Join-Path $PSScriptRoot "prepare-unsigned-windows-beta.ps1") `
            -PackageRoot $PackageRoot `
            -OutputRoot (Join-Path $PackageRoot "nested-output") `
            -Version $Version `
            -CommitSha ("a" * 40) `
            -Repository "airwiki/airwiki" `
            -WorkflowRunUrl "https://github.com/airwiki/airwiki/actions/runs/123" `
            -RetentionDays 30
    } "nested beta output"

    [IO.File]::WriteAllText((Join-Path $PackageRoot "unexpected.txt"), "unexpected")
    Assert-Rejected {
        & (Join-Path $PSScriptRoot "prepare-unsigned-windows-beta.ps1") `
            -PackageRoot $PackageRoot `
            -OutputRoot $OutputRoot `
            -Version $Version `
            -CommitSha ("a" * 40) `
            -Repository "airwiki/airwiki" `
            -WorkflowRunUrl "https://github.com/airwiki/airwiki/actions/runs/123" `
            -RetentionDays 30
    } "unexpected package input"
} finally {
    Remove-AirWikiWindowsStagingPath `
        -Path $TestRoot `
        -AllowedRoot $TargetRoot `
        -Label "unsigned beta policy test"
}

Write-Host "Unsigned Windows beta artifact policy tests passed."
