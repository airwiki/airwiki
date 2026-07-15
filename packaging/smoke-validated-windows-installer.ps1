[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string] $Installer,

    [Parameter(Mandatory = $true)]
    [string] $BundleRoot,

    [switch] $AuthorizeDestructiveInstallerSmoke
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$StateWaitMilliseconds = 15000
$ModelReadyWaitMilliseconds = 900000
$McpRequestTimeoutSeconds = 8
$McpRequestId = 991

. (Join-Path $PSScriptRoot "windows-runtime.ps1")
. (Join-Path $PSScriptRoot "windows-payload.ps1")

function Invoke-Process(
    [string] $Path,
    [string[]] $Arguments,
    [string] $Label
) {
    $Process = Start-Process `
        -FilePath $Path `
        -ArgumentList $Arguments `
        -Wait `
        -PassThru
    $ExitCode = $Process.ExitCode
    $Process.Dispose()
    if ($ExitCode -ne 0) {
        throw "$Label returned a nonzero exit code"
    }
}

function Get-DesktopProcesses {
    return @(Get-CimInstance Win32_Process -Filter "Name = 'airwiki.exe'")
}

function Test-SamePath([string] $Left, [string] $Right) {
    if ([string]::IsNullOrWhiteSpace($Left) -or [string]::IsNullOrWhiteSpace($Right)) {
        return $false
    }
    return [IO.Path]::GetFullPath($Left).Equals(
        [IO.Path]::GetFullPath($Right),
        [StringComparison]::OrdinalIgnoreCase
    )
}

function Assert-NoForeignDesktopProcess([string] $ExpectedExecutable) {
    foreach ($Process in @(Get-DesktopProcesses)) {
        if (-not (Test-SamePath ([string] $Process.ExecutablePath) $ExpectedExecutable)) {
            throw "another AirWiki executable is running; close it before this smoke test"
        }
    }
}

function Remove-OuterQuotes([string] $Value) {
    $Text = $Value.Trim()
    if ($Text.Length -ge 2 -and $Text[0] -eq '"' -and $Text[$Text.Length - 1] -eq '"') {
        return $Text.Substring(1, $Text.Length - 2)
    }
    return $Text
}

function Get-ExactRegisteredUninstaller {
    if (-not (Test-Path -LiteralPath $InstallDir -PathType Container) -or
        -not (Test-Path -LiteralPath $UninstallRegistryPath) -or
        -not (Test-Path -LiteralPath $ProductRegistryPath)) {
        throw "the existing per-user installation is incomplete or conflicting"
    }

    $ExpectedDesktop = Join-Path $InstallDir "airwiki.exe"
    $ExpectedUninstaller = Join-Path $InstallDir "uninstall.exe"
    if (-not (Test-Path -LiteralPath $ExpectedDesktop -PathType Leaf) -or
        -not (Test-Path -LiteralPath $ExpectedUninstaller -PathType Leaf)) {
        throw "the existing per-user installation is incomplete or conflicting"
    }

    $Metadata = Get-ItemProperty -LiteralPath $UninstallRegistryPath
    $ProductKey = Get-Item -LiteralPath $ProductRegistryPath
    if ([string] $Metadata.DisplayName -cne "AirWiki" -or
        [string] $Metadata.Publisher -cne "AirWiki" -or
        -not (Test-SamePath (Remove-OuterQuotes ([string] $Metadata.InstallLocation)) $InstallDir) -or
        -not (Test-SamePath (Remove-OuterQuotes ([string] $Metadata.UninstallString)) $ExpectedUninstaller) -or
        -not (Test-SamePath ([string] $ProductKey.GetValue("")) $InstallDir)) {
        throw "the existing per-user installation is incomplete or conflicting"
    }

    Assert-NoForeignDesktopProcess $ExpectedDesktop
    return $ExpectedUninstaller
}

function Test-AnyManagedInstallState {
    return (Test-Path -LiteralPath $InstallDir) -or
        (Test-Path -LiteralPath $UninstallRegistryPath) -or
        (Test-Path -LiteralPath $ProductRegistryPath) -or
        (Test-ManagedAutostartState)
}

function Test-ManagedAutostartState {
    if (-not (Test-Path -LiteralPath $AutostartRegistryPath)) {
        return $false
    }
    $Key = Get-Item -LiteralPath $AutostartRegistryPath -ErrorAction Stop
    return @($Key.GetValueNames()) -contains $AutostartValueName
}

function Wait-ForManagedStateRemoval {
    $Deadline = [DateTime]::UtcNow.AddMilliseconds($StateWaitMilliseconds)
    do {
        if (-not (Test-AnyManagedInstallState) -and
            (@(Get-DesktopProcesses)).Count -eq 0) {
            return
        }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $Deadline)
    throw "the per-user uninstall left managed application state"
}

function Remove-ExactRegisteredInstall {
    $Uninstaller = Get-ExactRegisteredUninstaller
    Invoke-Process $Uninstaller @("/S") "uninstaller"
    Wait-ForManagedStateRemoval
}

function ConvertFrom-McpBody([string] $Body) {
    try {
        return $Body | ConvertFrom-Json -ErrorAction Stop
    } catch { }
    foreach ($Line in @($Body -split "`r?`n")) {
        if ($Line.StartsWith("data:")) {
            try {
                return $Line.Substring(5).TrimStart() | ConvertFrom-Json -ErrorAction Stop
            } catch { }
        }
    }
    throw "the local MCP endpoint returned an invalid response"
}

function Wait-ForModelsReady {
    $Curl = (Get-Command curl.exe -CommandType Application -ErrorAction Stop).Source
    $Body = @{
        jsonrpc = "2.0"
        id = $McpRequestId
        method = "tools/call"
        params = @{
            name = "search_airwiki"
            arguments = @{
                question = "diagnostico sintetico de disponibilidad local"
                top_k = 1
            }
        }
    } | ConvertTo-Json -Depth 6 -Compress
    $Deadline = [DateTime]::UtcNow.AddMilliseconds($ModelReadyWaitMilliseconds)
    do {
        $ResponseLines = @(& $Curl `
            --silent `
            --show-error `
            --noproxy "*" `
            --connect-timeout 2 `
            --max-time $McpRequestTimeoutSeconds `
            --header "Connection: close" `
            --header "Content-Type: application/json" `
            --header "Accept: application/json, text/event-stream" `
            --data-binary $Body `
            "http://127.0.0.1:43123/mcp" 2>$null)
        if ($LASTEXITCODE -eq 0) {
            try {
                $Envelope = ConvertFrom-McpBody ([string]::Join("`n", $ResponseLines))
                $ErrorProperty = $Envelope.PSObject.Properties["error"]
                $ResultProperty = $Envelope.PSObject.Properties["result"]
                $StructuredProperty = if ($null -ne $ResultProperty) {
                    $ResultProperty.Value.PSObject.Properties["structuredContent"]
                } else {
                    $null
                }
                if ($Envelope.id -eq $McpRequestId -and
                    $null -eq $ErrorProperty -and
                    $null -ne $StructuredProperty -and
                    $null -ne $StructuredProperty.Value) {
                    return
                }
            } catch { }
        }
        Start-Sleep -Seconds 3
    } while ([DateTime]::UtcNow -lt $Deadline)
    throw "the installed application did not make its local models operational in time"
}

if (-not $AuthorizeDestructiveInstallerSmoke) {
    throw "the validated installer smoke test requires explicit destructive authorization"
}
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT -or
    -not [Environment]::Is64BitProcess) {
    throw "the validated installer smoke test requires 64-bit Windows"
}
$Os = Get-CimInstance Win32_OperatingSystem
$Processors = @(Get-CimInstance Win32_Processor)
if ([int] $Os.ProductType -ne 1 -or [version] $Os.Version -lt [version] "10.0" -or
    $Processors.Count -eq 0 -or @($Processors | Where-Object Architecture -ne 9).Count -ne 0) {
    throw "the validated installer smoke test requires native x64 Windows 10 or 11 client"
}
if ([string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
    throw "Windows did not expose the per-user local application data directory"
}

$InstallerItem = Get-Item -LiteralPath $Installer -ErrorAction Stop
$BundleItem = Get-Item -LiteralPath $BundleRoot -ErrorAction Stop
if (-not $InstallerItem.PSIsContainer -and $InstallerItem.Extension -ieq ".exe") {
    $Installer = $InstallerItem.FullName
} else {
    throw "Installer must be one Windows executable"
}
if (-not $BundleItem.PSIsContainer) {
    throw "BundleRoot must be a directory"
}
$BundleRoot = $BundleItem.FullName
$InstallDir = Join-Path $env:LOCALAPPDATA "AirWiki"
$UninstallRegistryPath = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\AirWiki"
$ProductRegistryPath = "HKCU:\Software\AirWiki\AirWiki"
$AutostartRegistryPath = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
$AutostartValueName = "AirWiki"
$DesktopExecutable = Join-Path $InstallDir "airwiki.exe"
$BundleFiles = [ordered]@{
    "airwiki.exe" = Join-Path $BundleRoot "airwiki.exe"
    "integrations/bridge/airwiki-mcp-bridge.exe" = `
        Join-Path $BundleRoot "airwiki-mcp-bridge.exe"
    "airwiki-windows-firewall-helper.exe" = `
        Join-Path $BundleRoot "airwiki-windows-firewall-helper.exe"
    "llama/llama-server.exe" = Join-Path $BundleRoot "llama\llama-server.exe"
    "llama/BUILD-MANIFEST.json" = Join-Path $BundleRoot "llama\BUILD-MANIFEST.json"
}
foreach ($Entry in $BundleFiles.GetEnumerator()) {
    if (-not (Test-Path -LiteralPath $Entry.Value -PathType Leaf)) {
        throw "$($Entry.Key) is missing from the validated bundle"
    }
}

if ((Test-AnyManagedInstallState) -or (@(Get-DesktopProcesses)).Count -ne 0) {
    throw "the validated installer smoke test requires a clean initial state; remove the existing installation manually"
}

$InstalledByThisRun = $true
$Failure = $null
$CleanupFailure = $null
try {
    Invoke-Process $Installer @("/S", "/NS", "/R") "installer"
    $RegisteredUninstaller = Get-ExactRegisteredUninstaller

    $Deadline = [DateTime]::UtcNow.AddSeconds(30)
    do {
        Assert-NoForeignDesktopProcess $DesktopExecutable
        $DesktopProcesses = @(Get-DesktopProcesses)
        if ($DesktopProcesses.Count -eq 1) { break }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $Deadline)
    if ($DesktopProcesses.Count -ne 1) {
        throw "the installed desktop did not open from its per-user path"
    }

    $ExpectedInstalled = New-WindowsPayloadManifest
    foreach ($Entry in $BundleFiles.GetEnumerator()) {
        Add-WindowsPayloadFile `
            $ExpectedInstalled `
            $Entry.Key `
            $Entry.Value `
            "validated bundle file"
    }
    Add-WindowsPayloadFile `
        $ExpectedInstalled `
        "uninstall.exe" `
        $RegisteredUninstaller `
        "generated NSIS uninstaller"
    $ActualInstalled = Get-ActualWindowsPayloadManifest `
        $InstallDir `
        "installed application payload"
    Assert-WindowsPayloadManifestsEqual `
        $ExpectedInstalled `
        $ActualInstalled `
        "installed application payload"
    Wait-ForModelsReady

    Invoke-Process $RegisteredUninstaller @("/S") "uninstaller"
    Wait-ForManagedStateRemoval
    $InstalledByThisRun = $false
} catch {
    $Failure = $_
} finally {
    if ($InstalledByThisRun -and
        ((Test-AnyManagedInstallState) -or (@(Get-DesktopProcesses)).Count -ne 0)) {
        try {
            Remove-ExactRegisteredInstall
            $InstalledByThisRun = $false
        } catch {
            $CleanupFailure = "automatic cleanup was unsafe because the installed state was incomplete or conflicting; remove the partial per-user installation manually"
        }
    }
}

if ($null -ne $CleanupFailure) {
    $PrimaryFailure = if ($null -ne $Failure) {
        $Failure.Exception.Message
    } else {
        "the validated installer smoke test did not complete"
    }
    throw "$PrimaryFailure; $CleanupFailure"
}
if ($null -ne $Failure) {
    throw $Failure
}

$InstallerHash = (Get-FileHash -LiteralPath $Installer -Algorithm SHA256).Hash.ToLowerInvariant()
[Console]::Out.WriteLine(
    "WINDOWS_VALIDATED_INSTALLER_SMOKE_PASS installer_sha256=$InstallerHash models_ready=pass uninstall=pass"
)
