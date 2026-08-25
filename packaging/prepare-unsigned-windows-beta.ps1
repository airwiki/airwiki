[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $PackageRoot,

    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $OutputRoot,

    [Parameter(Mandatory = $true)]
    [ValidatePattern('^(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)$')]
    [string] $Version,

    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[0-9a-f]{40}$')]
    [string] $CommitSha,

    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$')]
    [string] $Repository,

    [Parameter(Mandatory = $true)]
    [ValidatePattern('^https:\/\/github\.com\/[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+\/actions\/runs\/[0-9]+$')]
    [string] $WorkflowRunUrl,

    [Parameter(Mandatory = $true)]
    [ValidateRange(1, 90)]
    [int] $RetentionDays,

    [DateTimeOffset] $GeneratedAtUtc = [DateTimeOffset]::UtcNow
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$TargetRoot = Join-Path $Root "target"
. (Join-Path $PSScriptRoot "windows-runtime.ps1")
. (Join-Path $PSScriptRoot "windows-safe-staging.ps1")

function Resolve-AirWikiBetaPath([string] $Path) {
    if ([IO.Path]::IsPathRooted($Path)) {
        return [IO.Path]::GetFullPath($Path)
    }
    return [IO.Path]::GetFullPath((Join-Path $Root $Path))
}

function Assert-AirWikiBetaChildPath(
    [string] $Path,
    [string] $AllowedRoot,
    [string] $Label
) {
    $Separators = [char[]] @(
        [IO.Path]::DirectorySeparatorChar,
        [IO.Path]::AltDirectorySeparatorChar
    )
    $ResolvedRoot = [IO.Path]::GetFullPath($AllowedRoot).TrimEnd($Separators)
    $ResolvedPath = [IO.Path]::GetFullPath($Path).TrimEnd($Separators)
    $Prefix = $ResolvedRoot + [IO.Path]::DirectorySeparatorChar
    if (-not $ResolvedPath.StartsWith($Prefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "$Label must stay inside the repository target directory"
    }
}

function Assert-AirWikiMsiHeader([string] $Path) {
    $Bytes = [IO.File]::ReadAllBytes($Path)
    $Expected = [byte[]] @(0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1)
    if ($Bytes.Length -lt $Expected.Length) {
        throw "unsigned beta installer is truncated"
    }
    for ($Index = 0; $Index -lt $Expected.Length; $Index += 1) {
        if ($Bytes[$Index] -ne $Expected[$Index]) {
            throw "unsigned beta input is not an MSI compound file"
        }
    }
}

if ($Repository -cne "airwiki/airwiki") {
    throw "unsigned beta artifacts are restricted to airwiki/airwiki"
}

New-Item -ItemType Directory -Path $TargetRoot -Force | Out-Null
$ResolvedTargetRoot = [IO.Path]::GetFullPath($TargetRoot)
$ResolvedPackageRoot = Resolve-AirWikiBetaPath $PackageRoot
$ResolvedOutputRoot = Resolve-AirWikiBetaPath $OutputRoot
Assert-AirWikiBetaChildPath $ResolvedPackageRoot $ResolvedTargetRoot "package input"
Assert-AirWikiBetaChildPath $ResolvedOutputRoot $ResolvedTargetRoot "beta output"
if ($ResolvedPackageRoot.Equals($ResolvedOutputRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw "package input and beta output must be different directories"
}
$DirectorySeparator = [string] [IO.Path]::DirectorySeparatorChar
$PackagePrefix = $ResolvedPackageRoot.TrimEnd([char[]] @('\', '/')) + $DirectorySeparator
$OutputPrefix = $ResolvedOutputRoot.TrimEnd([char[]] @('\', '/')) + $DirectorySeparator
if ($ResolvedOutputRoot.StartsWith($PackagePrefix, [StringComparison]::OrdinalIgnoreCase) -or
    $ResolvedPackageRoot.StartsWith($OutputPrefix, [StringComparison]::OrdinalIgnoreCase)) {
    throw "package input and beta output must not contain one another"
}

Assert-AirWikiWindowsPathHasNoReparsePoint $ResolvedTargetRoot "Windows target root"
Assert-AirWikiWindowsPathHasNoReparsePoint $ResolvedPackageRoot "unsigned beta package input"
Assert-AirWikiWindowsTreeHasNoReparsePoint $ResolvedPackageRoot "unsigned beta package input"
$PackageRootItem = Get-AirWikiWindowsPathItem `
    $ResolvedPackageRoot `
    "unsigned beta package input"
if ($null -eq $PackageRootItem -or -not $PackageRootItem.PSIsContainer) {
    throw "unsigned beta package input is missing or is not a directory"
}

$PackageEntries = @(Get-ChildItem -LiteralPath $ResolvedPackageRoot -Force)
$Installers = @(
    $PackageEntries |
        Where-Object { -not $_.PSIsContainer -and $_.Extension -ieq ".msi" } |
        Sort-Object -Property Name
)
if ($PackageEntries.Count -ne 2 -or $Installers.Count -ne 2) {
    throw "unsigned beta input must contain exactly two localized MSI installers"
}

foreach ($Installer in $Installers) {
    if ($Installer.Name.Length -gt 200 -or
        $Installer.Name -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]*\.msi$' -or
        $Installer.Name -notlike "*$Version*") {
        throw "unsigned beta installer has an invalid or mismatched file name"
    }
    $VerifiedInstaller = Get-VerifiedWindowsRegularFile `
        $Installer.FullName `
        "unsigned beta MSI installer"
    Assert-AirWikiMsiHeader $VerifiedInstaller
}

Remove-AirWikiWindowsStagingPath `
    -Path $ResolvedOutputRoot `
    -AllowedRoot $ResolvedTargetRoot `
    -Label "unsigned beta artifact staging"
New-Item -ItemType Directory -Path $ResolvedOutputRoot | Out-Null

$InstallerMetadata = @()
foreach ($Installer in $Installers) {
    $Destination = Join-Path $ResolvedOutputRoot $Installer.Name
    Copy-Item -LiteralPath $Installer.FullName -Destination $Destination
    $VerifiedDestination = Get-VerifiedWindowsRegularFile `
        $Destination `
        "staged unsigned beta MSI installer"
    $DestinationItem = Get-Item -LiteralPath $VerifiedDestination -Force
    $InstallerMetadata += [ordered] @{
        name = $DestinationItem.Name
        sha256 = (Get-WindowsFileSha256 $VerifiedDestination "staged beta MSI").ToLowerInvariant()
        bytes = $DestinationItem.Length
    }
}

$ChecksumLines = @(
    $InstallerMetadata |
        ForEach-Object { "$($_.sha256)  $($_.name)" }
)
[IO.File]::WriteAllText(
    (Join-Path $ResolvedOutputRoot "SHA256SUMS.txt"),
    (($ChecksumLines -join "`n") + "`n"),
    [Text.UTF8Encoding]::new($false)
)

$Generated = $GeneratedAtUtc.ToUniversalTime()
$Expires = $Generated.AddDays($RetentionDays)
$Provenance = [ordered] @{
    schema_version = 1
    artifact_kind = "airwiki-windows-unsigned-beta"
    supported_public_release = $false
    code_signing = "not-requested"
    repository = $Repository
    commit_sha = $CommitSha
    version = $Version
    workflow_run_url = $WorkflowRunUrl
    generated_at_utc = $Generated.ToString("yyyy-MM-ddTHH:mm:ssZ")
    expires_at_utc = $Expires.ToString("yyyy-MM-ddTHH:mm:ssZ")
    retention_days = $RetentionDays
    installers = $InstallerMetadata
}
[IO.File]::WriteAllText(
    (Join-Path $ResolvedOutputRoot "PROVENANCE.json"),
    (($Provenance | ConvertTo-Json -Depth 4) + "`n"),
    [Text.UTF8Encoding]::new($false)
)

$Instructions = @"
AIRWIKI WINDOWS X64 - UNSIGNED BETA / BETA SIN FIRMA

This is a temporary development candidate for invited technical testing. It is
not a supported public release, is not connected to automatic updates, and has
not been signed by SignPath. / Este es un candidato temporal para pruebas
técnicas invitadas. No es un release público soportado, no está conectado a las
actualizaciones automáticas y no fue firmado por SignPath.

BEFORE INSTALLING / ANTES DE INSTALAR

1. Confirm that this artifact came from:
   $WorkflowRunUrl
2. Confirm commit $CommitSha and version $Version in PROVENANCE.json.
3. Verify each MSI from PowerShell and compare it with SHA256SUMS.txt:
   Get-FileHash -Algorithm SHA256 .\<installer>.msi
4. Use a non-production test account or device and back up any existing AirWiki
   data needed for the test.

WINDOWS PROTECTION / PROTECCIÓN DE WINDOWS

Do not disable SmartScreen, Smart App Control, antivirus, or organization policy.
After verifying the hash, use a per-file confirmation only when Windows offers
one and the device owner or administrator permits unsigned beta software. If
Windows or organization policy blocks the installer, stop; do not weaken that
policy. / No desactives SmartScreen, Smart App Control, el antivirus ni las
políticas de la organización. Después de verificar el hash, usa una confirmación
para ese archivo únicamente si Windows la ofrece y el propietario o administrador
permite software beta sin firma. Si Windows o la organización bloquean el
instalador, detente y no debilites esa política.

Choose the MSI for the desired installer language. Quit AirWiki completely
before uninstalling. Local application data is intentionally preserved by the
uninstaller. Report reproducible problems through GitHub Issues without attaching
private documents, queries, identities, addresses, credentials, databases, or
raw logs.

This artifact expires from GitHub Actions after $RetentionDays days. A newer
artifact must be verified independently; never reuse checksums from this build.
"@
[IO.File]::WriteAllText(
    (Join-Path $ResolvedOutputRoot "UNSIGNED-BETA.txt"),
    ($Instructions.Trim() + "`n"),
    [Text.UTF8Encoding]::new($false)
)

Write-Host "Prepared unsigned Windows beta artifact for commit $CommitSha"
