[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string] $Installer,

    [string] $UpgradeInstaller,

    [switch] $AuthorizeDestructiveMsiSmoke,

    [switch] $AllowGitHubHostedWindowsServer2022
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$ProcessWaitMilliseconds = 300000
$StateWaitMilliseconds = 30000
$ProductName = "AirWiki"
$Publisher = "AirWiki"
$LocalDataRoot = [Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)
$RoamingDataRoot = [Environment]::GetFolderPath([Environment+SpecialFolder]::ApplicationData)
$ProgramsRoot = [Environment]::GetFolderPath([Environment+SpecialFolder]::Programs)
$InstallDirectory = Join-Path (Join-Path $LocalDataRoot "Programs") $ProductName
$ShortcutPath = Join-Path (Join-Path $ProgramsRoot $ProductName) "$ProductName.lnk"
$LocalMarkerPath = Join-Path $LocalDataRoot "airwiki\AirWiki\msi-smoke-marker.json"
$RoamingMarkerPath = Join-Path $RoamingDataRoot "airwiki\AirWiki\msi-smoke-marker.json"
$script:InstalledProduct = $null
$script:ManualCleanupRequired = $false
$script:Markers = [Collections.Generic.List[object]]::new()
$script:MarkerDirectories = [Collections.Generic.List[object]]::new()
. (Join-Path $PSScriptRoot "windows-msi-smoke-host-policy.ps1")
. (Join-Path $PSScriptRoot "windows-installer-record.ps1")

function Assert-RegularFile([string] $Path, [string] $Label) {
    $Item = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
    if ($Item.PSIsContainer -or
        ($Item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "$Label must be a regular file"
    }
    return $Item.FullName
}

function Get-CanonicalPath([string] $Path, [string] $Label) {
    if ([string]::IsNullOrWhiteSpace($Path)) { throw "$Label is empty" }
    return [IO.Path]::GetFullPath($Path).TrimEnd('\')
}

function Assert-PathInsideRoot([string] $Path, [string] $Root, [string] $Label) {
    $CanonicalRoot = Get-CanonicalPath $Root "$Label root"
    $CanonicalPath = Get-CanonicalPath $Path $Label
    if (-not $CanonicalPath.StartsWith("$CanonicalRoot\", [StringComparison]::OrdinalIgnoreCase)) {
        throw "$Label is outside its canonical Windows known-folder root"
    }
    $Cursor = $CanonicalPath
    while ($true) {
        if (Test-Path -LiteralPath $Cursor) {
            $Item = Get-Item -LiteralPath $Cursor -Force -ErrorAction Stop
            if (($Item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "$Label has a reparse-point ancestor"
            }
        }
        if ($Cursor.Equals($CanonicalRoot, [StringComparison]::OrdinalIgnoreCase)) { break }
        $Cursor = Split-Path -Parent $Cursor
        if ([string]::IsNullOrWhiteSpace($Cursor)) {
            throw "$Label could not be traced to its canonical root"
        }
    }
    return $CanonicalPath
}

function ConvertTo-MsiCommandLine([string[]] $Arguments) {
    return (($Arguments | ForEach-Object {
        if ($_ -match '[\s"]') {
            if ($_.Contains('"')) { throw "MSI argument contains an unsupported quote" }
            '"' + $_ + '"'
        } else { $_ }
    }) -join ' ')
}

function ConvertTo-MsiGuid([string] $Value, [string] $Label) {
    $Parsed = [guid]::Empty
    if (-not [guid]::TryParse($Value, [ref]$Parsed)) { throw "$Label is not a GUID" }
    return "{$($Parsed.ToString().ToUpperInvariant())}"
}

function New-OwnedMarkerDirectories([string] $Parent, [string] $Root) {
    $SafeParent = Assert-PathInsideRoot $Parent $Root "MSI smoke marker parent"
    $CanonicalRoot = Get-CanonicalPath $Root "MSI smoke marker root"
    $Relative = $SafeParent.Substring($CanonicalRoot.Length).TrimStart('\')
    $Current = $CanonicalRoot
    foreach ($Segment in $Relative.Split('\', [StringSplitOptions]::RemoveEmptyEntries)) {
        $Current = Join-Path $Current $Segment
        if (Test-Path -LiteralPath $Current) {
            $Item = Get-Item -LiteralPath $Current -Force -ErrorAction Stop
            if (-not $Item.PSIsContainer) { throw "MSI smoke marker parent is not a directory" }
            continue
        }
        New-Item -ItemType Directory -Path $Current -ErrorAction Stop | Out-Null
        $SafeCurrent = Assert-PathInsideRoot $Current $Root "MSI smoke marker directory"
        $script:MarkerDirectories.Add([pscustomobject]@{ Path = $SafeCurrent; Root = $CanonicalRoot }) | Out-Null
    }
}

function Remove-EmptyOwnedMarkerDirectories {
    for ($Index = $script:MarkerDirectories.Count - 1; $Index -ge 0; $Index--) {
        $Directory = $script:MarkerDirectories[$Index]
        $null = Assert-PathInsideRoot $Directory.Path $Directory.Root "owned MSI smoke marker directory"
        if (-not (Test-Path -LiteralPath $Directory.Path -PathType Container)) {
            $script:MarkerDirectories.RemoveAt($Index)
            continue
        }
        $Item = Get-Item -LiteralPath $Directory.Path -Force -ErrorAction Stop
        if (($Item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "owned MSI smoke marker directory became a reparse point"
        }
        if (@(Get-ChildItem -LiteralPath $Directory.Path -Force -ErrorAction Stop).Count -ne 0) {
            continue
        }
        Remove-Item -LiteralPath $Directory.Path -Force
        $script:MarkerDirectories.RemoveAt($Index)
    }
}

function Assert-WindowsMsiSmokeHost {
    $Os = Get-CimInstance Win32_OperatingSystem
    $Processors = @(Get-CimInstance Win32_Processor)
    $HostEnvironment = @{}
    foreach ($Name in @(
        "GITHUB_ACTIONS",
        "RUNNER_ENVIRONMENT",
        "RUNNER_OS",
        "RUNNER_ARCH",
        "GITHUB_SERVER_URL",
        "GITHUB_REPOSITORY",
        "GITHUB_EVENT_NAME",
        "GITHUB_REF",
        "GITHUB_REF_TYPE",
        "GITHUB_REF_NAME",
        "GITHUB_JOB",
        "GITHUB_WORKFLOW_REF",
        "GITHUB_SHA",
        "AIRWIKI_RELEASE_COMMIT"
    )) {
        $HostEnvironment[$Name] = [Environment]::GetEnvironmentVariable($Name)
    }
    Assert-WindowsMsiSmokeHostPolicy `
        -HasDestructiveAuthorization $AuthorizeDestructiveMsiSmoke.IsPresent `
        -AllowGitHubHostedWindowsServer2022 $AllowGitHubHostedWindowsServer2022.IsPresent `
        -IsWindows ([Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT) `
        -Is64BitProcess ([Environment]::Is64BitProcess) `
        -ProductType ([int]$Os.ProductType) `
        -OperatingSystemVersion ([version]$Os.Version) `
        -BuildNumber ([string]$Os.BuildNumber) `
        -ProcessorArchitectures ([int[]]@($Processors | ForEach-Object { [int]$_.Architecture })) `
        -Environment $HostEnvironment
    if ([string]::IsNullOrWhiteSpace($LocalDataRoot) -or
        [string]::IsNullOrWhiteSpace($RoamingDataRoot) -or
        [string]::IsNullOrWhiteSpace($ProgramsRoot)) {
        throw "the MSI smoke test requires current-user data roots"
    }
    $null = Assert-PathInsideRoot $InstallDirectory $LocalDataRoot "MSI installation directory"
    $null = Assert-PathInsideRoot $ShortcutPath $ProgramsRoot "Start Menu shortcut"
    $null = Assert-PathInsideRoot $LocalMarkerPath $LocalDataRoot "local marker"
    $null = Assert-PathInsideRoot $RoamingMarkerPath $RoamingDataRoot "roaming marker"
}

function Get-MsiProperties([string] $Path) {
    $InstallerCom = $null
    $Database = $null
    $View = $null
    try {
        $InstallerCom = New-Object -ComObject WindowsInstaller.Installer
        $Database = $InstallerCom.OpenDatabase($Path, 0)
        $View = $Database.OpenView('SELECT `Property`, `Value` FROM `Property`')
        $View.Execute()
        $Properties = @{}
        while ($true) {
            $Record = $View.Fetch()
            if ($null -eq $Record) { break }
            try {
                $PropertyName = Get-WindowsInstallerRecordStringData -Record $Record -Field 1
                $PropertyValue = Get-WindowsInstallerRecordStringData -Record $Record -Field 2
                $Properties[$PropertyName] = $PropertyValue
            } finally {
                [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($Record)
            }
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
        ProductCode = ConvertTo-MsiGuid ([string]$Properties.ProductCode) "MSI ProductCode"
        UpgradeCode = ConvertTo-MsiGuid ([string]$Properties.UpgradeCode) "MSI UpgradeCode"
        ProductVersion = $Version
        Sha256 = (Get-FileHash -LiteralPath $Verified -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}

function Get-ArpPath([string] $ProductCode) {
    return "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$ProductCode"
}

function Get-ProductRegistrationPaths([string] $ProductCode) {
    return @(
        "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$ProductCode",
        "HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$ProductCode",
        "HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\$ProductCode"
    ) | Where-Object { Test-Path -LiteralPath $_ }
}

function Assert-NoProductCodeRegistration($Metadata) {
    if ((Get-ProductRegistrationPaths $Metadata.ProductCode).Count -ne 0) {
        throw "the MSI ProductCode already has a registered installation; resolve it manually"
    }
}

function Assert-MsiProductNotInstalled($Metadata) {
    $InstallerCom = $null
    try {
        $InstallerCom = New-Object -ComObject WindowsInstaller.Installer
        $State = [int]$InstallerCom.ProductState($Metadata.ProductCode)
    } finally {
        if ($null -ne $InstallerCom) { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($InstallerCom) }
    }
    if ($State -ne -1 -and $State -ne 2) {
        throw "the MSI ProductCode is already known to Windows Installer (state $State); resolve it manually"
    }
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

function Assert-CleanPreflight($Metadata) {
    $null = Assert-PathInsideRoot $InstallDirectory $LocalDataRoot "MSI installation directory"
    $null = Assert-PathInsideRoot $ShortcutPath $ProgramsRoot "Start Menu shortcut"
    if (-not (Test-WebView2Present)) {
        throw "WebView2 Runtime is absent; refusing an installer path that could download it"
    }
    if ((Get-AirWikiArpEntries).Count -ne 0 -or
        (Test-Path -LiteralPath $InstallDirectory) -or
        (Test-Path -LiteralPath $ShortcutPath)) {
        throw "the MSI smoke test requires no existing AirWiki installer, payload, or shortcut collision"
    }
    Assert-NoProductCodeRegistration $Metadata
    Assert-MsiProductNotInstalled $Metadata
    if (@(Get-CimInstance Win32_Process -Filter "Name = 'airwiki.exe'").Count -ne 0) {
        throw "close AirWiki before the MSI smoke test"
    }
}

function Get-MsiOperationState([string] $ProductCode) {
    $Registrations = @(Get-ProductRegistrationPaths $ProductCode).Count
    $MsiExecProcesses = @(Get-CimInstance Win32_Process -Filter "Name = 'msiexec.exe'").Count
    return "registrations=$Registrations payload=$([bool](Test-Path -LiteralPath $InstallDirectory)) shortcut=$([bool](Test-Path -LiteralPath $ShortcutPath)) msiexec_processes=$MsiExecProcesses"
}

function Invoke-MsiExec([string[]] $Arguments, [string] $Label, [string] $ProductCode) {
    $MsiExec = Assert-RegularFile (Join-Path $env:SystemRoot "System32\msiexec.exe") "Windows Installer executable"
    $CommandLine = ConvertTo-MsiCommandLine $Arguments
    $Process = Start-Process -FilePath $MsiExec -ArgumentList $CommandLine -PassThru -WindowStyle Hidden
    try {
        if (-not $Process.WaitForExit($ProcessWaitMilliseconds)) {
            try { $Process.Kill() } catch { }
            $null = $Process.WaitForExit(10000)
            $script:ManualCleanupRequired = $true
            throw "$Label timed out; Windows Installer may still be changing state ($((Get-MsiOperationState $ProductCode))). Preserve state and clean it manually"
        }
        if ($Process.ExitCode -ne 0) {
            $State = Get-MsiOperationState $ProductCode
            if ($State -notmatch 'registrations=0 payload=False shortcut=False') {
                $script:ManualCleanupRequired = $true
            }
            throw "$Label returned exit $($Process.ExitCode) ($State)"
        }
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
        $Path = Join-Path $Root $Relative
        $null = Assert-PathInsideRoot $Path $LocalDataRoot "installed payload $Relative"
        $null = Assert-RegularFile $Path "installed payload $Relative"
    }
}

function Assert-InstalledProduct($Metadata) {
    $SafeInstallDirectory = Assert-PathInsideRoot $InstallDirectory $LocalDataRoot "MSI installation directory"
    $SafeShortcutPath = Assert-PathInsideRoot $ShortcutPath $ProgramsRoot "Start Menu shortcut"
    Wait-ForArp $Metadata.ProductCode $true
    $Arp = Get-ItemProperty -LiteralPath (Get-ArpPath $Metadata.ProductCode) -ErrorAction Stop
    if ([string]$Arp.DisplayName -cne $ProductName -or [string]$Arp.Publisher -cne $Publisher -or
        -not (Test-SamePath ([string]$Arp.InstallLocation) $SafeInstallDirectory)) {
        throw "MSI ARP metadata does not identify the installed AirWiki payload"
    }
    Assert-EssentialPayload $SafeInstallDirectory
    $DesktopExecutable = Join-Path $SafeInstallDirectory "airwiki.exe"
    $null = Assert-PathInsideRoot $DesktopExecutable $LocalDataRoot "installed desktop executable"
    $null = Assert-RegularFile $DesktopExecutable "installed desktop executable"
    $null = Assert-RegularFile $SafeShortcutPath "Start Menu shortcut"
    $Shell = New-Object -ComObject WScript.Shell
    try {
        $Shortcut = $Shell.CreateShortcut($SafeShortcutPath)
        if (-not (Test-SamePath ([string]$Shortcut.TargetPath) $DesktopExecutable)) {
            throw "Start Menu shortcut target is not the installed desktop executable"
        }
    } finally { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($Shell) }
    return $true
}

function New-Marker([string] $Path, $Metadata) {
    $Root = if ($Path -ceq $LocalMarkerPath) {
        $LocalDataRoot
    } elseif ($Path -ceq $RoamingMarkerPath) {
        $RoamingDataRoot
    } else {
        throw "MSI smoke marker path is not owned by this smoke test"
    }
    $SafePath = Assert-PathInsideRoot $Path $Root "MSI smoke marker"
    if (Test-Path -LiteralPath $SafePath) { throw "MSI smoke marker path already exists" }
    $Parent = Split-Path -Parent $SafePath
    New-OwnedMarkerDirectories $Parent $Root
    $SafePath = Assert-PathInsideRoot $SafePath $Root "MSI smoke marker"
    $Record = [ordered]@{
        schema = 1
        run_id = [guid]::NewGuid().ToString("D")
        installer_sha256 = $Metadata.Sha256
        product_code = $Metadata.ProductCode
    } | ConvertTo-Json -Compress
    $Stream = [IO.File]::Open($SafePath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try {
        $Writer = [IO.StreamWriter]::new($Stream, [Text.UTF8Encoding]::new($false))
        try { $Writer.Write($Record) } finally { $Writer.Dispose() }
    } finally { $Stream.Dispose() }
    $script:Markers.Add([pscustomobject]@{ Path = $SafePath; Root = $Root; Sha256 = (Get-FileHash -LiteralPath $SafePath -Algorithm SHA256).Hash }) | Out-Null
}

function Assert-MarkersPreserved {
    foreach ($Marker in $script:Markers) {
        $null = Assert-PathInsideRoot $Marker.Path $Marker.Root "owned MSI smoke marker"
        $null = Assert-RegularFile $Marker.Path "owned MSI smoke marker"
        $Actual = (Get-FileHash -LiteralPath $Marker.Path -Algorithm SHA256).Hash
        if ($Actual -cne $Marker.Sha256) { throw "MSI uninstall changed an owned data-root marker" }
    }
}

function Remove-ExactMarkers {
    foreach ($Marker in @($script:Markers)) {
        $null = Assert-PathInsideRoot $Marker.Path $Marker.Root "owned MSI smoke marker"
        $null = Assert-RegularFile $Marker.Path "owned MSI smoke marker"
        if (-not (Test-Path -LiteralPath $Marker.Path -PathType Leaf)) { throw "owned marker disappeared before cleanup" }
        if ((Get-FileHash -LiteralPath $Marker.Path -Algorithm SHA256).Hash -cne $Marker.Sha256) {
            throw "owned marker changed; refusing deletion"
        }
        Remove-Item -LiteralPath $Marker.Path -Force
        $script:Markers.Remove($Marker) | Out-Null
    }
    Remove-EmptyOwnedMarkerDirectories
}

function Remove-InstalledProduct($Metadata) {
    try {
        $null = Assert-InstalledProduct $Metadata
        Invoke-MsiExec @("/x", $Metadata.ProductCode, "/qn", "/norestart") "MSI uninstall" $Metadata.ProductCode
        Wait-ForArp $Metadata.ProductCode $false
        if ((Get-ProductRegistrationPaths $Metadata.ProductCode).Count -ne 0) {
            throw "MSI uninstall left a ProductCode registration"
        }
        $null = Assert-PathInsideRoot $InstallDirectory $LocalDataRoot "MSI installation directory"
        $null = Assert-PathInsideRoot $ShortcutPath $ProgramsRoot "Start Menu shortcut"
        if ((Test-Path -LiteralPath $InstallDirectory) -or (Test-Path -LiteralPath $ShortcutPath)) {
            throw "MSI uninstall left the application payload or Start Menu shortcut"
        }
    } catch {
        $script:ManualCleanupRequired = $true
        throw
    }
    $script:InstalledProduct = $null
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
    Assert-CleanPreflight $Base
    if ($null -ne $Upgrade -and $Upgrade.ProductCode -cne $Base.ProductCode) {
        Assert-NoProductCodeRegistration $Upgrade
        Assert-MsiProductNotInstalled $Upgrade
    }
    New-Marker $LocalMarkerPath $Base
    New-Marker $RoamingMarkerPath $Base
    Invoke-MsiExec @("/i", $Base.Path, "/qn", "/norestart", "AUTOLAUNCHAPP=0") "MSI install" $Base.ProductCode
    try { $null = Assert-InstalledProduct $Base } catch {
        $script:ManualCleanupRequired = $true
        throw
    }
    $script:InstalledProduct = $Base

    if ($null -ne $Upgrade) {
        try { $null = Assert-InstalledProduct $Base } catch {
            $script:ManualCleanupRequired = $true
            throw
        }
        if ($Upgrade.ProductCode -cne $Base.ProductCode) {
            Assert-NoProductCodeRegistration $Upgrade
            Assert-MsiProductNotInstalled $Upgrade
        }
        Invoke-MsiExec @("/i", $Upgrade.Path, "/qn", "/norestart", "AUTOLAUNCHAPP=0") "MSI upgrade" $Upgrade.ProductCode
        if ($Upgrade.ProductCode -cne $Base.ProductCode) {
            Wait-ForArp $Base.ProductCode $false
        }
        try { $null = Assert-InstalledProduct $Upgrade } catch {
            $script:ManualCleanupRequired = $true
            throw
        }
        $script:InstalledProduct = $Upgrade
    }

    Remove-InstalledProduct $script:InstalledProduct
    Assert-MarkersPreserved
    Remove-ExactMarkers
    Write-Host "Windows MSI smoke passed."
} finally {
    if ($script:ManualCleanupRequired) {
        [Console]::Error.WriteLine("MSI smoke preserved installer and marker state for manual cleanup after an ambiguous operation.")
    } elseif ($null -ne $script:InstalledProduct) {
        try { Remove-InstalledProduct $script:InstalledProduct } catch {
            [Console]::Error.WriteLine("MSI smoke cleanup could not uninstall its verified product: $($_.Exception.Message)")
        }
    }
    if ($script:Markers.Count -ne 0 -and $null -eq $script:InstalledProduct -and -not $script:ManualCleanupRequired) {
        try { Remove-ExactMarkers } catch {
            [Console]::Error.WriteLine("MSI smoke cleanup preserved changed or missing marker state: $($_.Exception.Message)")
        }
    }
    if ($script:MarkerDirectories.Count -ne 0 -and -not $script:ManualCleanupRequired) {
        try { Remove-EmptyOwnedMarkerDirectories } catch {
            [Console]::Error.WriteLine("MSI smoke cleanup preserved marker-directory state: $($_.Exception.Message)")
        }
    }
}
