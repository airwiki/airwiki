[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $BundleRoot,

    [string] $SevenZipPath
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    throw "Validated Windows bundle packaging requires Windows"
}

$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$Config = Join-Path $Root "packaging\windows\ValidatedBundle.Packager.toml"
$LlamaPolicy = Join-Path $Root "packaging\llama-windows-build-policy.json"
$TargetRoot = Join-Path $Root "target"
$WorkRoot = Join-Path $TargetRoot "validated-windows-bundle"
$StageRoot = Join-Path $WorkRoot "staging"
$ExtractRoot = Join-Path $WorkRoot "extracted"
$OutDir = Join-Path $TargetRoot "packages\windows-validated"
$NsisRoot = Join-Path `
    ([Environment]::GetFolderPath("LocalApplicationData")) `
    ".cargo-packager\NSIS"

. (Join-Path $PSScriptRoot "windows-runtime.ps1")
. (Join-Path $PSScriptRoot "windows-payload.ps1")
. (Join-Path $PSScriptRoot "windows-safe-staging.ps1")

function New-BundleTreeManifest(
    [string] $Desktop,
    [string] $Bridge,
    [string] $Helper,
    [string] $LlamaServer,
    [string] $LlamaManifest,
    [string] $Label
) {
    $Manifest = New-WindowsPayloadManifest
    Add-WindowsPayloadFile $Manifest "airwiki.exe" $Desktop "$Label desktop"
    Add-WindowsPayloadFile `
        $Manifest `
        "airwiki-mcp-bridge.exe" `
        $Bridge `
        "$Label MCP bridge"
    Add-WindowsPayloadFile `
        $Manifest `
        "airwiki-windows-firewall-helper.exe" `
        $Helper `
        "$Label firewall helper"
    Add-WindowsPayloadFile `
        $Manifest `
        "llama/llama-server.exe" `
        $LlamaServer `
        "$Label llama-server"
    Add-WindowsPayloadFile `
        $Manifest `
        "llama/BUILD-MANIFEST.json" `
        $LlamaManifest `
        "$Label llama.cpp build manifest"
    return $Manifest
}

function New-InstalledCoreManifest(
    [string] $Desktop,
    [string] $Bridge,
    [string] $Helper,
    [string] $LlamaServer,
    [string] $LlamaManifest,
    [string] $Label
) {
    $Manifest = New-WindowsPayloadManifest
    Add-WindowsPayloadFile $Manifest "airwiki.exe" $Desktop "$Label desktop"
    Add-WindowsPayloadFile `
        $Manifest `
        "integrations/bridge/airwiki-mcp-bridge.exe" `
        $Bridge `
        "$Label MCP bridge"
    Add-WindowsPayloadFile `
        $Manifest `
        "airwiki-windows-firewall-helper.exe" `
        $Helper `
        "$Label firewall helper"
    Add-WindowsPayloadFile `
        $Manifest `
        "llama/llama-server.exe" `
        $LlamaServer `
        "$Label llama-server"
    Add-WindowsPayloadFile `
        $Manifest `
        "llama/BUILD-MANIFEST.json" `
        $LlamaManifest `
        "$Label llama.cpp build manifest"
    return $Manifest
}

function Get-SingleExtractedFile([string] $RootPath, [string] $Name) {
    $Matches = @(Get-ChildItem -LiteralPath $RootPath -Recurse -File -Filter $Name)
    if ($Matches.Count -ne 1) {
        throw "Expected exactly one $Name in the NSIS payload"
    }
    return $Matches[0].FullName
}

function Assert-ExactExtractedNsisPayload(
    [object] $ExpectedCore,
    [string] $PayloadRoot,
    [string] $Label
) {
    $Actual = Get-ActualWindowsPayloadManifest $PayloadRoot $Label
    $ExpectedFiles = @($ExpectedCore.Files.Keys) + @(
        '$PLUGINSDIR/System.dll',
        '$PLUGINSDIR/modern-wizard.bmp',
        '$PLUGINSDIR/nsDialogs.dll',
        '$PLUGINSDIR/nsis_tauri_utils.dll',
        '$PLUGINSDIR/StartMenu.dll',
        '$PLUGINSDIR/LangDLL.dll',
        'uninstall.exe'
    )
    $ExpectedDirectories = @($ExpectedCore.Directories.Keys) + @('$PLUGINSDIR')
    if ($Actual.Files.Count -ne $ExpectedFiles.Count -or
        $Actual.Directories.Count -ne $ExpectedDirectories.Count) {
        throw "$Label contains unexpected files or directories"
    }
    foreach ($Relative in $ExpectedFiles) {
        if (-not $Actual.Files.ContainsKey($Relative)) {
            throw "$Label is missing $Relative"
        }
    }
    foreach ($Relative in $ExpectedDirectories) {
        if (-not $Actual.Directories.ContainsKey($Relative)) {
            throw "$Label is missing $Relative"
        }
    }
    foreach ($Relative in $ExpectedCore.Files.Keys) {
        if ($ExpectedCore.Files[$Relative].Length -ne $Actual.Files[$Relative].Length -or
            $ExpectedCore.Files[$Relative].Sha256 -ne $Actual.Files[$Relative].Sha256) {
            throw "$Label contains product bytes that differ from the validated bundle"
        }
    }
    Assert-WindowsPeMachine `
        (Join-Path $PayloadRoot 'uninstall.exe') `
        0x014c `
        "$Label uninstaller"
}

function Resolve-SevenZip([string] $RequestedPath) {
    $Candidates = [Collections.Generic.List[string]]::new()
    if (-not [string]::IsNullOrWhiteSpace($RequestedPath)) {
        $ResolvedRequested = if ([IO.Path]::IsPathRooted($RequestedPath)) {
            [IO.Path]::GetFullPath($RequestedPath)
        } else {
            [IO.Path]::GetFullPath((Join-Path (Get-Location).Path $RequestedPath))
        }
        $Candidates.Add($ResolvedRequested)
    } else {
        if (-not [string]::IsNullOrWhiteSpace($env:ProgramFiles)) {
            $Candidates.Add((Join-Path $env:ProgramFiles "7-Zip\7z.exe"))
        }
        $ProgramFilesX86 = ${env:ProgramFiles(x86)}
        if (-not [string]::IsNullOrWhiteSpace($ProgramFilesX86)) {
            $Candidates.Add((Join-Path $ProgramFilesX86 "7-Zip\7z.exe"))
        }
        $Candidates.Add((Join-Path $Root "target\verified-tools\7zip-26.02\7z.exe"))
    }

    foreach ($Candidate in $Candidates) {
        if (Test-Path -LiteralPath $Candidate -PathType Leaf) {
            return Get-VerifiedWindowsRegularFile $Candidate "7-Zip inspection executable"
        }
    }
    throw "7-Zip is required only to inspect the generated installer; install it or pass -SevenZipPath"
}

function Assert-PreparedNsisCache([string] $CacheRoot) {
    foreach ($Relative in @(
        "makensis.exe",
        "Bin\makensis.exe",
        "Stubs\lzma-x86-unicode",
        "Stubs\lzma_solid-x86-unicode",
        "Plugins\x86-unicode\ApplicationID.dll",
        "Plugins\x86-unicode\nsis_tauri_utils.dll",
        "Include\MUI2.nsh"
    )) {
        $null = Get-VerifiedWindowsRegularFile `
            (Join-Path $CacheRoot $Relative) `
            "prepared NSIS cache entry"
    }
    $ApplicationIdSentinel = Get-Item -LiteralPath `
        (Join-Path $CacheRoot "Plugins\x86-unicode\ApplicationID.dll") `
        -Force
    if ($ApplicationIdSentinel.Length -ne 0) {
        throw "Prepared NSIS cache has an unexpected ApplicationID compatibility file"
    }
}

$CandidateBundleRoot = if ([IO.Path]::IsPathRooted($BundleRoot)) {
    [IO.Path]::GetFullPath($BundleRoot)
} else {
    [IO.Path]::GetFullPath((Join-Path (Get-Location).Path $BundleRoot))
}
if (-not (Test-Path -LiteralPath $CandidateBundleRoot -PathType Container)) {
    throw "Validated bundle root is missing or is not a directory: $CandidateBundleRoot"
}
$BundleItem = Get-Item -LiteralPath $CandidateBundleRoot -Force -ErrorAction Stop
Assert-NoWindowsReparseAncestor $BundleItem.FullName "validated bundle root"
$ResolvedBundleRoot = $BundleItem.FullName
$Separators = [char[]]@(
    [IO.Path]::DirectorySeparatorChar,
    [IO.Path]::AltDirectorySeparatorChar
)
$BundleBoundary = [IO.Path]::GetFullPath($ResolvedBundleRoot).TrimEnd($Separators)
$BundlePrefix = $BundleBoundary + [IO.Path]::DirectorySeparatorChar
foreach ($ManagedRoot in @($WorkRoot, $OutDir)) {
    $ManagedBoundary = [IO.Path]::GetFullPath($ManagedRoot).TrimEnd($Separators)
    $ManagedPrefix = $ManagedBoundary + [IO.Path]::DirectorySeparatorChar
    if ($BundleBoundary.Equals($ManagedBoundary, [StringComparison]::OrdinalIgnoreCase) -or
        $BundleBoundary.StartsWith($ManagedPrefix, [StringComparison]::OrdinalIgnoreCase) -or
        $ManagedBoundary.StartsWith($BundlePrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Validated bundle root overlaps managed packaging output"
    }
}

$BundleDesktop = Join-Path $ResolvedBundleRoot "airwiki.exe"
$BundleBridge = Join-Path $ResolvedBundleRoot "airwiki-mcp-bridge.exe"
$BundleHelper = Join-Path $ResolvedBundleRoot "airwiki-windows-firewall-helper.exe"
$BundleLlamaServer = Join-Path $ResolvedBundleRoot "llama\llama-server.exe"
$BundleLlamaManifest = Join-Path $ResolvedBundleRoot "llama\BUILD-MANIFEST.json"
$ExpectedBundleTree = New-BundleTreeManifest `
    $BundleDesktop `
    $BundleBridge `
    $BundleHelper `
    $BundleLlamaServer `
    $BundleLlamaManifest `
    "validated bundle"
$ActualBundleTree = Get-ActualWindowsPayloadManifest `
    $ResolvedBundleRoot `
    "validated bundle"
Assert-WindowsPayloadManifestsEqual `
    $ExpectedBundleTree `
    $ActualBundleTree `
    "validated bundle"
Assert-WindowsPeMachine $BundleDesktop 0x8664 "validated bundle desktop"
Assert-WindowsPeMachine $BundleBridge 0x8664 "validated bundle MCP bridge"
Assert-WindowsPeMachine $BundleHelper 0x8664 "validated bundle firewall helper"
Assert-WindowsPeMachine $BundleLlamaServer 0x8664 "validated bundle llama-server"
$null = Get-WindowsLlamaRuntimeManifest `
    (Join-Path $ResolvedBundleRoot "llama") `
    $LlamaPolicy
Assert-WindowsDesktopEmbedsLlamaRuntimeHash `
    $BundleDesktop `
    (Join-Path $ResolvedBundleRoot "llama") `
    $LlamaPolicy
Assert-WindowsFirewallHelperManifest $BundleHelper "validated bundle firewall helper"

$SevenZip = Resolve-SevenZip $SevenZipPath
Assert-PreparedNsisCache $NsisRoot
$CargoPackager = (Get-Command cargo-packager.exe -CommandType Application `
    -ErrorAction Stop).Source
$CargoPackagerVersion = (& $CargoPackager --version 2>&1 | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $CargoPackagerVersion -ne "cargo-packager 0.11.8") {
    throw "cargo-packager 0.11.8 is required"
}

Push-Location $Root
try {
    Remove-AirWikiWindowsStagingPath `
        -Path $WorkRoot `
        -AllowedRoot $TargetRoot `
        -Label "validated Windows bundle staging"
    New-Item -ItemType Directory -Path (Join-Path $StageRoot "llama") -Force | Out-Null
    Copy-Item -LiteralPath $BundleDesktop -Destination (Join-Path $StageRoot "airwiki.exe")
    Copy-Item `
        -LiteralPath $BundleBridge `
        -Destination (Join-Path $StageRoot "airwiki-mcp-bridge.exe")
    Copy-Item `
        -LiteralPath $BundleHelper `
        -Destination (Join-Path $StageRoot "airwiki-windows-firewall-helper.exe")
    Copy-Item `
        -LiteralPath $BundleLlamaServer `
        -Destination (Join-Path $StageRoot "llama\llama-server.exe")
    Copy-Item `
        -LiteralPath $BundleLlamaManifest `
        -Destination (Join-Path $StageRoot "llama\BUILD-MANIFEST.json")

    $StagedTree = Get-ActualWindowsPayloadManifest $StageRoot "staged validated bundle"
    Assert-WindowsPayloadManifestsEqual `
        $ExpectedBundleTree `
        $StagedTree `
        "staged validated bundle"

    Remove-AirWikiWindowsStagingPath `
        -Path $OutDir `
        -AllowedRoot $TargetRoot `
        -Label "validated Windows package output"
    New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
    $Started = [DateTime]::UtcNow
    & $CargoPackager --config $Config
    if ($LASTEXITCODE -ne 0) {
        throw "cargo-packager failed"
    }

    $Installers = @(Get-ChildItem -LiteralPath $OutDir -File -Filter *.exe)
    if ($Installers.Count -ne 1) {
        throw "Expected exactly one fresh NSIS installer"
    }
    if ($Installers[0].LastWriteTimeUtc -lt $Started) {
        throw "NSIS installer predates this packaging run"
    }
    Assert-WindowsPeMachine $Installers[0].FullName 0x014c "fresh NSIS installer"

    New-Item -ItemType Directory -Path $ExtractRoot -Force | Out-Null
    & $SevenZip x -y "-o$ExtractRoot" $Installers[0].FullName | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "7-Zip could not inspect the NSIS payload"
    }

    $PackagedDesktop = Get-SingleExtractedFile $ExtractRoot "airwiki.exe"
    $PayloadRoot = [IO.Path]::GetDirectoryName($PackagedDesktop)
    $PackagedBridge = Get-VerifiedWindowsRegularFile `
        (Join-Path $PayloadRoot "integrations\bridge\airwiki-mcp-bridge.exe") `
        "packaged MCP bridge"
    $PackagedHelper = Get-VerifiedWindowsRegularFile `
        (Join-Path $PayloadRoot "airwiki-windows-firewall-helper.exe") `
        "packaged firewall helper"
    $PackagedLlamaServer = Get-VerifiedWindowsRegularFile `
        (Join-Path $PayloadRoot "llama\llama-server.exe") `
        "packaged llama-server"
    $PackagedLlamaManifest = Get-VerifiedWindowsRegularFile `
        (Join-Path $PayloadRoot "llama\BUILD-MANIFEST.json") `
        "packaged llama.cpp build manifest"
    $ExpectedInstalledCore = New-InstalledCoreManifest `
        $BundleDesktop `
        $BundleBridge `
        $BundleHelper `
        $BundleLlamaServer `
        $BundleLlamaManifest `
        "validated bundle"
    Assert-ExactExtractedNsisPayload `
        $ExpectedInstalledCore `
        $PayloadRoot `
        "extracted NSIS payload"

    $FinalBundleTree = Get-ActualWindowsPayloadManifest `
        $ResolvedBundleRoot `
        "validated bundle after packaging"
    Assert-WindowsPayloadManifestsEqual `
        $ExpectedBundleTree `
        $FinalBundleTree `
        "validated bundle after packaging"
    Write-Host "Verified NSIS from the validated Windows bundle: $($Installers[0].FullName)"
} finally {
    Remove-AirWikiWindowsStagingPath `
        -Path $WorkRoot `
        -AllowedRoot $TargetRoot `
        -Label "validated Windows bundle staging"
    Pop-Location
}
