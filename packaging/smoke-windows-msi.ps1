[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string] $Installer,

    [string] $UpgradeInstaller,

    [switch] $AuthorizeDestructiveMsiSmoke
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$ProcessWaitMilliseconds = 300000
$StateWaitMilliseconds = 30000
$ProductName = "AirWiki"
$Publisher = "AirWiki"
$InstallDirectory = Join-Path (Join-Path $env:LOCALAPPDATA "Programs") $ProductName
$ShortcutPath = Join-Path (Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\AirWiki") "AirWiki.lnk"
$LocalMarkerPath = Join-Path $env:LOCALAPPDATA "airwiki\AirWiki\msi-smoke-marker.json"
$RoamingMarkerPath = Join-Path $env:APPDATA "airwiki\AirWiki\msi-smoke-marker.json"
$script:InstalledProductCode = $null
$script:Markers = [Collections.Generic.List[object]]::new()

function Assert-RegularFile([string] $Path, [string] $Label) {
    $Item = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
    if ($Item.PSIsContainer -or
        ($Item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "$Label must be a regular file"
    }
    return $Item.FullName
}

function Assert-WindowsMsiSmokeHost {
    if (-not $AuthorizeDestructiveMsiSmoke) {
        throw "the MSI smoke test requires explicit destructive authorization"
    }
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT -or
        -not [Environment]::Is64BitProcess) {
        throw "the MSI smoke test requires 64-bit Windows"
    }
    $Os = Get-CimInstance Win32_OperatingSystem
    $Processors = @(Get-CimInstance Win32_Processor)
    if ([int]$Os.ProductType -ne 1 -or [version]$Os.Version -lt [version]"10.0" -or
        $Processors.Count -eq 0 -or @($Processors | Where-Object Architecture -ne 9).Count -ne 0) {
        throw "the MSI smoke test requires native x64 Windows 10 or 11 client"
    }
    if ([string]::IsNullOrWhiteSpace($env:LOCALAPPDATA) -or
        [string]::IsNullOrWhiteSpace($env:APPDATA)) {
        throw "the MSI smoke test requires current-user data roots"
    }
}

function Get-MsiProperties([string] $Path) {
    $InstallerCom = $null
    $Database = $null
    $View = $null
    try {
        $InstallerCom = New-Object -ComObject WindowsInstaller.Installer
        $Database = $InstallerCom.OpenDatabase($Path, 0)
        $View = $Database.OpenView("SELECT `Property`, `Value` FROM `Property`")
        $View.Execute()
        $Properties = @{}
        while ($true) {
            $Record = $View.Fetch()
            if ($null -eq $Record) { break }
            $Properties[[string]$Record.StringData(1)] = [string]$Record.StringData(2)
            [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($Record)
        }
        return $Properties
    } finally {
        if ($null -ne $View) { $View.Close(); [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($View) }
        if ($null -ne $Database) { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($Database) }
        if ($null -ne $InstallerCom) { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($InstallerCom) }
    }
}

function Get-InstallerMetadata([string] $Path) {
    $Verified = Assert-RegularFile $Path "MSI installer"
    $Properties = Get-MsiProperties $Verified
    foreach ($Name in @("ProductCode", "UpgradeCode", "ProductVersion", "ProductName", "Manufacturer")) {
        if ([string]::IsNullOrWhiteSpace([string]$Properties[$Name])) {
            throw "MSI installer is missing $Name metadata"
        }
    }
    if ([string]$Properties.ProductName -cne $ProductName -or
        [string]$Properties.Manufacturer -cne $Publisher) {
        throw "MSI installer has an unexpected product identity"
    }
    try { $Version = [version][string]$Properties.ProductVersion } catch { throw "MSI ProductVersion is invalid" }
    return [pscustomobject]@{
        Path = $Verified
        ProductCode = [string]$Properties.ProductCode
        UpgradeCode = [string]$Properties.UpgradeCode
        ProductVersion = $Version
        Sha256 = (Get-FileHash -LiteralPath $Verified -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}

function Get-ArpPath([string] $ProductCode) {
    return "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$ProductCode"
}

function Test-WebView2Present {
    $Paths = @(
        "HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
        "HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
        "HKCU:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"
    )
    foreach ($Path in $Paths) {
        if (Test-Path -LiteralPath $Path) {
            $Metadata = Get-ItemProperty -LiteralPath $Path -Name pv -ErrorAction SilentlyContinue
            $Version = [string]$Metadata.pv
            if (-not [string]::IsNullOrWhiteSpace($Version)) { return $true }
        }
    }
    return $false
}

function Get-AirWikiArpEntries {
    $Root = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall"
    if (-not (Test-Path -LiteralPath $Root)) { return @() }
    return @(Get-ChildItem -LiteralPath $Root -ErrorAction Stop | ForEach-Object {
        $Metadata = Get-ItemProperty -LiteralPath $_.PSPath -ErrorAction Stop
        if ([string]$Metadata.DisplayName -ceq $ProductName -or
            [string]$Metadata.Publisher -ceq $Publisher) {
            [pscustomobject]@{ Path = $_.PSPath; DisplayName = [string]$Metadata.DisplayName; Publisher = [string]$Metadata.Publisher }
        }
    })
}

function Assert-CleanPreflight {
    if (-not (Test-WebView2Present)) {
        throw "WebView2 Runtime is absent; refusing an installer path that could download it"
    }
    if ((Get-AirWikiArpEntries).Count -ne 0 -or
        (Test-Path -LiteralPath $InstallDirectory) -or
        (Test-Path -LiteralPath $ShortcutPath)) {
        throw "the MSI smoke test requires no existing AirWiki installer, payload, or shortcut collision"
    }
    if (@(Get-CimInstance Win32_Process -Filter "Name = 'airwiki.exe'").Count -ne 0) {
        throw "close AirWiki before the MSI smoke test"
    }
}

function Invoke-MsiExec([string[]] $Arguments, [string] $Label) {
    $MsiExec = Assert-RegularFile (Join-Path $env:SystemRoot "System32\msiexec.exe") "Windows Installer executable"
    $Process = Start-Process -FilePath $MsiExec -ArgumentList $Arguments -PassThru -WindowStyle Hidden
    try {
        if (-not $Process.WaitForExit($ProcessWaitMilliseconds)) {
            $Process.Kill()
            if (-not $Process.WaitForExit(10000)) { throw "$Label timeout cleanup did not complete" }
            throw "$Label did not exit within the bounded wait"
        }
        if ($Process.ExitCode -ne 0) { throw "$Label returned exit $($Process.ExitCode)" }
    } finally { $Process.Dispose() }
}

function Test-SamePath([string] $Left, [string] $Right) {
    return [IO.Path]::GetFullPath($Left).TrimEnd('\').Equals(
        [IO.Path]::GetFullPath($Right).TrimEnd('\'),
        [StringComparison]::OrdinalIgnoreCase
    )
}

function Wait-ForArp([string] $ProductCode, [bool] $Present) {
    $Path = Get-ArpPath $ProductCode
    $Deadline = [DateTime]::UtcNow.AddMilliseconds($StateWaitMilliseconds)
    do {
        if ((Test-Path -LiteralPath $Path) -eq $Present) { return }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $Deadline)
    throw "MSI ARP state did not reach the expected value"
}

function Assert-EssentialPayload([string] $Root) {
    foreach ($Relative in @(
        "airwiki.exe",
        "integrations\bridge\airwiki-mcp-bridge.exe",
        "airwiki-windows-firewall-helper.exe",
        "integrations\airwiki-claude.mcpb",
        "llama\llama-server.exe",
        "LICENSE",
        "THIRD_PARTY_NOTICES.md"
    )) {
        $null = Assert-RegularFile (Join-Path $Root $Relative) "installed payload $Relative"
    }
}

function Assert-InstalledProduct($Metadata) {
    Wait-ForArp $Metadata.ProductCode $true
    $Arp = Get-ItemProperty -LiteralPath (Get-ArpPath $Metadata.ProductCode) -ErrorAction Stop
    if ([string]$Arp.DisplayName -cne $ProductName -or [string]$Arp.Publisher -cne $Publisher -or
        -not (Test-SamePath ([string]$Arp.InstallLocation) $InstallDirectory)) {
        throw "MSI ARP metadata does not identify the installed AirWiki payload"
    }
    Assert-EssentialPayload $InstallDirectory
    $Shell = New-Object -ComObject WScript.Shell
    try {
        $Shortcut = $Shell.CreateShortcut($ShortcutPath)
        if (-not (Test-SamePath ([string]$Shortcut.TargetPath) (Join-Path $InstallDirectory "airwiki.exe"))) {
            throw "Start Menu shortcut target is not the installed desktop executable"
        }
    } finally { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($Shell) }
}

function New-Marker([string] $Path, $Metadata) {
    if (Test-Path -LiteralPath $Path) { throw "MSI smoke marker path already exists" }
    $Parent = Split-Path -Parent $Path
    New-Item -ItemType Directory -Path $Parent -Force | Out-Null
    $Record = [ordered]@{
        schema = 1
        run_id = [guid]::NewGuid().ToString("D")
        installer_sha256 = $Metadata.Sha256
        product_code = $Metadata.ProductCode
    } | ConvertTo-Json -Compress
    [IO.File]::WriteAllText($Path, $Record, [Text.UTF8Encoding]::new($false))
    $script:Markers.Add([pscustomobject]@{ Path = $Path; Sha256 = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash }) | Out-Null
}

function Assert-MarkersPreserved {
    foreach ($Marker in $script:Markers) {
        $Actual = (Get-FileHash -LiteralPath $Marker.Path -Algorithm SHA256).Hash
        if ($Actual -cne $Marker.Sha256) { throw "MSI uninstall changed an owned data-root marker" }
    }
}

function Remove-ExactMarkers {
    foreach ($Marker in @($script:Markers)) {
        if (-not (Test-Path -LiteralPath $Marker.Path -PathType Leaf)) { throw "owned marker disappeared before cleanup" }
        if ((Get-FileHash -LiteralPath $Marker.Path -Algorithm SHA256).Hash -cne $Marker.Sha256) {
            throw "owned marker changed; refusing deletion"
        }
        Remove-Item -LiteralPath $Marker.Path -Force
        $script:Markers.Remove($Marker) | Out-Null
    }
}

function Remove-InstalledProduct([string] $ProductCode) {
    Invoke-MsiExec @("/x", $ProductCode, "/qn", "/norestart") "MSI uninstall"
    Wait-ForArp $ProductCode $false
    if ((Test-Path -LiteralPath $InstallDirectory) -or (Test-Path -LiteralPath $ShortcutPath)) {
        throw "MSI uninstall left the application payload or Start Menu shortcut"
    }
    $script:InstalledProductCode = $null
}

try {
    Assert-WindowsMsiSmokeHost
    $Base = Get-InstallerMetadata $Installer
    $Upgrade = $null
    if (-not [string]::IsNullOrWhiteSpace($UpgradeInstaller)) {
        $Upgrade = Get-InstallerMetadata $UpgradeInstaller
        if ($Upgrade.UpgradeCode -cne $Base.UpgradeCode -or $Upgrade.ProductVersion -le $Base.ProductVersion) {
            throw "upgrade MSI must keep UpgradeCode and have a strictly greater ProductVersion"
        }
    }
    Assert-CleanPreflight
    New-Marker $LocalMarkerPath $Base
    New-Marker $RoamingMarkerPath $Base
    Invoke-MsiExec @("/i", $Base.Path, "/qn", "/norestart", "AUTOLAUNCHAPP=0") "MSI install"
    $script:InstalledProductCode = $Base.ProductCode
    Assert-InstalledProduct $Base

    if ($null -ne $Upgrade) {
        Invoke-MsiExec @("/i", $Upgrade.Path, "/qn", "/norestart", "AUTOLAUNCHAPP=0") "MSI upgrade"
        if ($Upgrade.ProductCode -cne $Base.ProductCode) {
            Wait-ForArp $Base.ProductCode $false
        }
        $script:InstalledProductCode = $Upgrade.ProductCode
        Assert-InstalledProduct $Upgrade
    }

    Remove-InstalledProduct $script:InstalledProductCode
    Assert-MarkersPreserved
    Remove-ExactMarkers
    Write-Host "Windows MSI smoke passed."
} finally {
    if ($null -ne $script:InstalledProductCode) {
        try { Remove-InstalledProduct $script:InstalledProductCode } catch { Write-Error "MSI smoke cleanup could not uninstall its verified product: $($_.Exception.Message)" }
    }
    if ($script:Markers.Count -ne 0 -and $null -eq $script:InstalledProductCode) {
        try { Remove-ExactMarkers } catch { Write-Error "MSI smoke cleanup preserved changed or missing marker state: $($_.Exception.Message)" }
    }
}
