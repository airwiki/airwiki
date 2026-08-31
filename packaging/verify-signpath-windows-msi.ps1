[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $PackageRoot,

    [string] $ExpectedVersion = $env:AIRWIKI_RELEASE_VERSION
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    throw "SignPath MSI verification requires Windows"
}

$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
. (Join-Path $PSScriptRoot "windows-release-version.ps1")
$ReleaseVersion = Get-AirWikiReleaseVersion $Root $ExpectedVersion
$ExpectedWindowsVersion = "$ReleaseVersion.0"
$TargetRoot = Join-Path $Root "target"
$LlamaPolicy = Join-Path $Root "packaging\llama-windows-build-policy.json"
$CandidateRoot = if ([IO.Path]::IsPathRooted($PackageRoot)) {
    [IO.Path]::GetFullPath($PackageRoot)
} else {
    [IO.Path]::GetFullPath((Join-Path (Get-Location).Path $PackageRoot))
}

. (Join-Path $PSScriptRoot "windows-payload.ps1")
. (Join-Path $PSScriptRoot "windows-runtime.ps1")
. (Join-Path $PSScriptRoot "windows-safe-staging.ps1")
. (Join-Path $PSScriptRoot "windows-signpath.ps1")

function Expand-WindowsMsi([string] $Installer, [string] $Destination) {
    $MsiExec = Get-VerifiedWindowsRegularFile `
        (Join-Path $env:SystemRoot "System32\msiexec.exe") `
        "Windows Installer executable"
    $Arguments = "/a `"$Installer`" /qn /norestart TARGETDIR=`"$Destination`""
    $Process = Start-Process `
        -FilePath $MsiExec `
        -ArgumentList $Arguments `
        -Wait `
        -PassThru `
        -WindowStyle Hidden
    if ($Process.ExitCode -ne 0) {
        throw "Windows Installer could not extract the signed MSI (exit $($Process.ExitCode))"
    }
}

function Assert-ExactWindowsVersion([string] $Path, [string] $Label) {
    $Info = (Get-Item -LiteralPath $Path).VersionInfo
    $FileVersion = "{0}.{1}.{2}.{3}" -f `
        $Info.FileMajorPart, $Info.FileMinorPart, $Info.FileBuildPart, $Info.FilePrivatePart
    $ProductVersion = "{0}.{1}.{2}.{3}" -f `
        $Info.ProductMajorPart, $Info.ProductMinorPart, $Info.ProductBuildPart, $Info.ProductPrivatePart
    if ($FileVersion -ne $script:ExpectedWindowsVersion -or
        $ProductVersion -ne $script:ExpectedWindowsVersion -or
        $Info.FileVersion -ne $script:ReleaseVersion -or
        $Info.ProductVersion -ne $script:ReleaseVersion) {
        throw "$Label version metadata does not exactly match $script:ReleaseVersion"
    }
}

if (-not (Test-Path -LiteralPath $CandidateRoot -PathType Container)) {
    throw "signed MSI package root is missing"
}
Assert-NoWindowsReparseAncestor $CandidateRoot "signed MSI package root"
$Installers = @(Get-ChildItem -LiteralPath $CandidateRoot -File -Filter *.msi)
$ExpectedInstallerNames = @(
    "AirWiki_${ReleaseVersion}_x64_en-US.msi",
    "AirWiki_${ReleaseVersion}_x64_es-ES.msi"
)
if ($Installers.Count -ne 2 -or
    @($Installers | Where-Object { $_.Name -cnotin $ExpectedInstallerNames }).Count -ne 0) {
    throw "signed package must contain exactly the en-US and es-ES AirWiki MSI files"
}
$ExpectedFiles = $ExpectedInstallerNames + @($ExpectedInstallerNames | ForEach-Object { "$_.sig" })
$Unexpected = @(Get-ChildItem -LiteralPath $CandidateRoot -Force | Where-Object {
    $_.PSIsContainer -or $_.Name -cnotin $ExpectedFiles
})
if ($Unexpected.Count -ne 0) {
    throw "signed MSI package root contains an unexpected file"
}

$ReferenceSigner = $null
$ReferencePayload = $null
$VerificationRoot = Join-Path $TargetRoot "signpath\signed-msi-verification"
Push-Location $Root
try {
    & cargo run --locked -p xtask -- packaging verify-windows-msi
    if ($LASTEXITCODE -ne 0) { throw "Windows MSI policy validation failed" }

    foreach ($Installer in $Installers) {
        $UpdaterSignature = Get-VerifiedWindowsRegularFile `
            "$($Installer.FullName).sig" `
            "Tauri updater signature"
        & cargo run --locked -p xtask -- packaging verify-updater-signature `
            --artifact $Installer.FullName `
            --signature $UpdaterSignature
        if ($LASTEXITCODE -ne 0) {
            throw "Tauri updater signature verification failed"
        }
        $MsiSigner = Get-VerifiedSignPathSignature $Installer.FullName "signed AirWiki MSI"
        Assert-ExpectedSignPathSigner $MsiSigner
        if ($null -eq $ReferenceSigner) {
            $ReferenceSigner = $MsiSigner
        } else {
            Assert-SameSignPathSigner $ReferenceSigner $MsiSigner "localized MSI"
        }

        Remove-AirWikiWindowsStagingPath `
            -Path $VerificationRoot `
            -AllowedRoot $TargetRoot `
            -Label "signed MSI verification staging"
        New-Item -ItemType Directory -Path $VerificationRoot -Force | Out-Null
        Expand-WindowsMsi $Installer.FullName $VerificationRoot

        $DesktopMatches = @(Get-ChildItem -LiteralPath $VerificationRoot -Recurse -File -Filter airwiki.exe)
        if ($DesktopMatches.Count -ne 1) {
            throw "signed MSI must contain exactly one AirWiki desktop"
        }
        $Desktop = $DesktopMatches[0].FullName
        Assert-ExactWindowsVersion $Desktop "signed MSI desktop"
        $PayloadRoot = $DesktopMatches[0].Directory.FullName
        $Bridge = Get-VerifiedWindowsRegularFile `
            (Join-Path $PayloadRoot "integrations\bridge\airwiki-mcp-bridge.exe") `
            "signed MSI MCP bridge"
        $Helper = Get-VerifiedWindowsRegularFile `
            (Join-Path $PayloadRoot "airwiki-windows-firewall-helper.exe") `
            "signed MSI firewall helper"
        $Mcpb = Get-VerifiedWindowsRegularFile `
            (Join-Path $PayloadRoot "integrations\airwiki-claude.mcpb") `
            "signed MSI Claude MCPB"
        $LlamaServerMatches = @(Get-ChildItem -LiteralPath $PayloadRoot -Recurse -File -Filter llama-server.exe)
        if ($LlamaServerMatches.Count -ne 1) {
            throw "signed MSI must contain exactly one llama runtime"
        }
        $RuntimeRoot = Get-WindowsPackagedRuntimeRoot $Desktop $LlamaServerMatches[0].FullName

        Assert-WindowsMsiBundleType $Desktop "signed MSI desktop"
        Assert-WindowsFirewallHelperManifest $Helper "signed MSI firewall helper"
        foreach ($Nested in @(
            @{ Path = $Desktop; Label = "desktop" },
            @{ Path = $Bridge; Label = "MCP bridge" },
            @{ Path = $Helper; Label = "firewall helper" }
        )) {
            $NestedSigner = Get-VerifiedSignPathSignature $Nested.Path "signed MSI $($Nested.Label)"
            Assert-ExpectedSignPathSigner $NestedSigner
            Assert-SameSignPathSigner $ReferenceSigner $NestedSigner $Nested.Label
        }

        $null = Get-WindowsLlamaRuntimeManifest $RuntimeRoot $LlamaPolicy
        Assert-WindowsDesktopEmbedsLlamaRuntimeHash $Desktop $RuntimeRoot $LlamaPolicy
        Assert-WindowsDirectoryTreeMatches `
            (Join-Path $Root "resources\integrations\workflow") `
            (Join-Path $PayloadRoot "integrations\workflow") `
            "signed MSI AirWiki workflow guide"
        & cargo run --locked -p xtask -- mcpb verify `
            --target x86_64-pc-windows-msvc `
            --bridge $Bridge `
            --output $Mcpb
        if ($LASTEXITCODE -ne 0) { throw "signed MSI Claude MCPB failed validation" }

        $CurrentPayload = New-WindowsPayloadManifest
        Add-WindowsPayloadTree $CurrentPayload "payload" $PayloadRoot "localized MSI payload"
        if ($null -eq $ReferencePayload) {
            $ReferencePayload = $CurrentPayload
        } else {
            if ($ReferencePayload.Files.Count -ne $CurrentPayload.Files.Count -or
                $ReferencePayload.Directories.Count -ne $CurrentPayload.Directories.Count) {
                throw "localized MSI payloads have different layouts"
            }
            foreach ($Relative in $ReferencePayload.Files.Keys) {
                if (-not $CurrentPayload.Files.ContainsKey($Relative) -or
                    $ReferencePayload.Files[$Relative].Length -ne $CurrentPayload.Files[$Relative].Length -or
                    $ReferencePayload.Files[$Relative].Sha256 -ne $CurrentPayload.Files[$Relative].Sha256) {
                    throw "localized MSI payloads contain different product bytes"
                }
            }
        }
    }
    Write-Host "Verified two signed AirWiki MSI packages and all nested product binaries"
} finally {
    Remove-AirWikiWindowsStagingPath `
        -Path $VerificationRoot `
        -AllowedRoot $TargetRoot `
        -Label "signed MSI verification staging"
    Pop-Location
}

