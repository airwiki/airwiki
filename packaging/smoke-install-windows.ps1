param([switch] $AuthorizeDestructiveClientInstallerGate)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$CurrentCase = "bootstrap"
$OwnerMarkerCreated = $false
$UninstallerVerified = $false
$UninstallerPassed = $false
$OwnedDesktopProcess = $null
$UpdaterRestartCleanupAllowed = $false
$InstallCleanupAllowed = $false
$OwnedRegistryFixtures = [Collections.Generic.List[object]]::new()
$CleanupFailures = [Collections.Generic.List[string]]::new()
$InstallerWaitMilliseconds = 120000
$ProcessCleanupWaitMilliseconds = 10000
$RejectedObserverSequence = 0

function Assert-WindowsClientInstallerGateHost {
    if (-not $AuthorizeDestructiveClientInstallerGate) {
        throw "the destructive Windows client installer gate requires explicit authorization"
    }
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
        throw "the real installer matrix requires Windows"
    }
    $Os = Get-CimInstance Win32_OperatingSystem
    $Processor = @(Get-CimInstance Win32_Processor)
    if ([int]$Os.ProductType -ne 1 -or [version]$Os.Version -lt [version]"10.0") {
        throw "the real installer matrix requires Windows 10/11 client, never Windows Server"
    }
    if ($Processor.Count -eq 0 -or @($Processor | Where-Object Architecture -ne 9).Count -ne 0) {
        throw "the real installer matrix requires native AMD64 processors"
    }
    if (-not [Environment]::Is64BitProcess) {
        throw "the real installer matrix requires a 64-bit PowerShell process"
    }
    $Identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $Principal = [Security.Principal.WindowsPrincipal]::new($Identity)
    if (-not $Principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw "the registry-fixture matrix requires an elevated clean client"
    }
    return [string]$Os.Version
}

function Assert-InstallerArguments([string[]] $Arguments) {
    if ($Arguments.Count -eq 0 -or
        $Arguments[$Arguments.Count - 1] -cne "/D=$InstallDir") {
        throw "the NSIS install directory must be the final argument"
    }
}

function Invoke-Installer([string[]] $Arguments, [int] $ExpectedExit, [string] $CaseId) {
    $script:CurrentCase = $CaseId
    Assert-InstallerArguments $Arguments
    Assert-MutationProcessPrecondition $CaseId
    $Process = Start-Process -FilePath $Installer -ArgumentList $Arguments -PassThru
    try {
        if (-not $Process.WaitForExit($InstallerWaitMilliseconds)) {
            $Process.Kill()
            if (-not $Process.WaitForExit($ProcessCleanupWaitMilliseconds)) {
                throw "installer timeout cleanup did not complete"
            }
            throw "installer did not exit within the bounded wait"
        }
        $ExitCode = $Process.ExitCode
    } finally {
        $Process.Dispose()
    }
    if ($ExitCode -ne $ExpectedExit) {
        throw "installer exit code did not match the case contract"
    }
}

function Set-InstalledVersionFixture([AllowEmptyString()][string] $Version) {
    if ($Version.Length -eq 0) {
        Assert-MutationProcessPrecondition $CurrentCase
        Remove-ItemProperty -LiteralPath $UninstallRegistryPath -Name DisplayVersion -ErrorAction Stop
    } else {
        Assert-MutationProcessPrecondition $CurrentCase
        Set-ItemProperty -LiteralPath $UninstallRegistryPath -Name DisplayVersion -Value $Version
    }
}

function Get-RegistryFingerprint([string] $Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return "absent" }
    $Key = Get-Item -LiteralPath $Path
    $Subkeys = @($Key.GetSubKeyNames() | Sort-Object)
    $Values = foreach ($Name in @($Key.GetValueNames() | Sort-Object)) {
        [ordered]@{
            name = $Name
            kind = [string]$Key.GetValueKind($Name)
            value = [string]$Key.GetValue(
                $Name,
                $null,
                [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames
            )
        }
    }
    return ([ordered]@{ subkeys = @($Subkeys); values = @($Values) } |
        ConvertTo-Json -Compress -Depth 5)
}

function Assert-OwnedRegistryMarker($Record) {
    if ([string]$Record.Owner -cne $RegistryOwner) {
        throw "owned registry fixture record has an unexpected owner"
    }
    $Key = Get-Item -LiteralPath $Record.Path -ErrorAction Stop
    $MarkerValue = [string]$Key.GetValue(
        "AirWikiTestOwner",
        $null,
        [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames
    )
    $MarkerKind = $Key.GetValueKind("AirWikiTestOwner")
    if ($MarkerValue -cne $RegistryOwner -or
        $MarkerKind -ne [Microsoft.Win32.RegistryValueKind]::String) {
        throw "owned registry fixture marker is missing or invalid"
    }
}

function New-OwnedWixFixture([string] $Label, [string] $Version) {
    $script:CurrentCase = "create-owned-wix-fixture"
    $KeyName = "AirWiki-$RegistryOwner-$Label"
    $Path = Join-Path $WixUninstallRoot $KeyName
    if (Test-Path -LiteralPath $Path) { throw "owned WiX fixture already exists" }
    Assert-MutationProcessPrecondition $CurrentCase
    New-Item -Path $Path -ErrorAction Stop | Out-Null
    $Record = [pscustomobject]@{
        Path = $Path
        Owner = $RegistryOwner
        MarkerTrusted = $false
        Fingerprint = $null
    }
    $OwnedRegistryFixtures.Add($Record) | Out-Null
    Assert-MutationProcessPrecondition $CurrentCase
    New-ItemProperty `
        -LiteralPath $Path `
        -Name AirWikiTestOwner `
        -Value $RegistryOwner `
        -PropertyType String | Out-Null
    Assert-OwnedRegistryMarker $Record
    $Record.MarkerTrusted = $true
    $Record.Fingerprint = Get-RegistryFingerprint $Path
    Assert-MutationProcessPrecondition $CurrentCase
    New-ItemProperty -LiteralPath $Path -Name DisplayName -Value "AirWiki" | Out-Null
    $Record.Fingerprint = Get-RegistryFingerprint $Path
    Assert-MutationProcessPrecondition $CurrentCase
    New-ItemProperty -LiteralPath $Path -Name Publisher -Value $ExpectedPublisher | Out-Null
    $Record.Fingerprint = Get-RegistryFingerprint $Path
    Assert-MutationProcessPrecondition $CurrentCase
    New-ItemProperty -LiteralPath $Path -Name DisplayVersion -Value $Version | Out-Null
    $Record.Fingerprint = Get-RegistryFingerprint $Path
    Assert-MutationProcessPrecondition $CurrentCase
    New-ItemProperty -LiteralPath $Path -Name UninstallString -Value "msiexec.exe /x {$RegistryOwner}" | Out-Null
    $Record.Fingerprint = Get-RegistryFingerprint $Path
    return $Record
}

function Remove-OwnedRegistryFixture($Record) {
    $script:CurrentCase = "remove-owned-wix-fixture"
    $OwnedRecordCount = @($OwnedRegistryFixtures | Where-Object {
        [object]::ReferenceEquals($_, $Record)
    }).Count
    $OwnedPathCount = @($OwnedRegistryFixtures | Where-Object {
        [string]$_.Path -ceq [string]$Record.Path
    }).Count
    if ($OwnedRecordCount -ne 1 -or $OwnedPathCount -ne 1) {
        throw "owned registry fixture record is missing or duplicated"
    }
    if ($Record.MarkerTrusted -ne $true -or $null -eq $Record.Fingerprint) {
        throw "owned registry fixture never acquired deletion authority"
    }
    Assert-OwnedRegistryMarker $Record
    if ((Get-RegistryFingerprint $Record.Path) -cne $Record.Fingerprint) {
        throw "owned registry fixture changed; refusing deletion"
    }
    Assert-MutationProcessPrecondition $CurrentCase
    Remove-Item -LiteralPath $Record.Path -Recurse -Force
    $OwnedRegistryFixtures.Remove($Record) | Out-Null
}

function Get-DesktopProcessesAtExactPath([string] $ExpectedPath) {
    $ExpectedFullPath = [IO.Path]::GetFullPath($ExpectedPath)
    $Matches = [Collections.Generic.List[object]]::new()
    foreach ($Process in @(Get-CimInstance Win32_Process)) {
        $ProcessPath = [string]$Process.ExecutablePath
        if ([string]::IsNullOrWhiteSpace($ProcessPath)) { continue }
        $ActualPath = [IO.Path]::GetFullPath($ProcessPath)
        if ($ActualPath.Equals($ExpectedFullPath, [StringComparison]::OrdinalIgnoreCase)) {
            $Matches.Add([pscustomobject]@{
                ProcessId = [uint32]$Process.ProcessId
                ExecutablePath = $ActualPath
            }) | Out-Null
        }
    }
    return @($Matches)
}

function Assert-NoDesktopProcess([string] $CaseId) {
    $script:CurrentCase = $CaseId
    if (@(Get-DesktopProcessesAtExactPath $DesktopExecutable).Count -ne 0) {
        throw "the case observed an unexpected desktop process"
    }
}

function Assert-MutationProcessPrecondition([string] $CaseId) {
    Assert-NoDesktopProcess $CaseId
}

function Start-RejectedDesktopProcessObserver {
    $Watcher = $null
    $Subscription = $null
    try {
        $script:RejectedObserverSequence++
        $SourceIdentifier = "AirWikiInstallerGateProcessStart-$RejectedObserverSequence"
        $Query = [System.Management.WqlEventQuery]::new(
            "SELECT * FROM Win32_ProcessStartTrace WHERE ProcessName = 'airwiki.exe'"
        )
        $Watcher = [System.Management.ManagementEventWatcher]::new($Query)
        Register-ObjectEvent `
            -InputObject $Watcher `
            -EventName EventArrived `
            -SourceIdentifier $SourceIdentifier `
            -ErrorAction Stop | Out-Null
        $Subscriptions = @(Get-EventSubscriber -ErrorAction Stop | Where-Object {
            $_.SourceIdentifier -ceq $SourceIdentifier
        })
        if ($Subscriptions.Count -ne 1) {
            throw "rejected-process observer subscription setup was ambiguous"
        }
        $Subscription = $Subscriptions[0]
        $Watcher.Start()
        return [pscustomobject]@{
            Watcher = $Watcher
            Subscription = $Subscription
            SourceIdentifier = $SourceIdentifier
        }
    } catch {
        @(Get-EventSubscriber -ErrorAction SilentlyContinue | Where-Object {
            $_.SourceIdentifier -ceq $SourceIdentifier
        }) | ForEach-Object {
            Unregister-Event -SubscriptionId $_.SubscriptionId -ErrorAction SilentlyContinue
        }
        if ($null -ne $Watcher) {
            try { $Watcher.Stop() } catch { }
            $Watcher.Dispose()
        }
        throw "rejected-process observer setup failed"
    }
}

function Get-RejectedDesktopProcessObserverEvents($Record) {
    if ($null -eq $Record -or
        $null -eq $Record.Watcher -or
        $null -eq $Record.Subscription) {
        throw "rejected-process observer record is incomplete"
    }
    $Subscribers = @(Get-EventSubscriber -ErrorAction Stop | Where-Object {
        $_.SourceIdentifier -ceq $Record.SourceIdentifier
    })
    if ($Subscribers.Count -ne 1 -or
        $Subscribers[0].SubscriptionId -ne $Record.Subscription.SubscriptionId) {
        throw "rejected-process observer subscription changed"
    }
    return @(Get-Event -ErrorAction Stop | Where-Object {
        $_.SourceIdentifier -ceq $Record.SourceIdentifier
    })
}

function Assert-RejectedDesktopProcessObserverClear($Record) {
    $null = Wait-Event -SourceIdentifier $Record.SourceIdentifier -Timeout 1
    if (@(Get-RejectedDesktopProcessObserverEvents $Record).Count -ne 0) {
        throw "a rejected installer started the desktop process"
    }
}

function Stop-RejectedDesktopProcessObserver($Record) {
    $TeardownFailed = $false
    $Observed = $false
    try {
        $Record.Watcher.Stop()
    } catch {
        $TeardownFailed = $true
    }
    try {
        $null = Wait-Event -SourceIdentifier $Record.SourceIdentifier -Timeout 1
    } catch {
        $TeardownFailed = $true
    }
    try {
        if (@(Get-RejectedDesktopProcessObserverEvents $Record).Count -ne 0) {
            $Observed = $true
        }
    } catch {
        $TeardownFailed = $true
    }
    try {
        Unregister-Event `
            -SubscriptionId $Record.Subscription.SubscriptionId `
            -ErrorAction Stop
    } catch {
        $TeardownFailed = $true
    }
    try {
        $FinalEvents = @(Get-Event -ErrorAction Stop | Where-Object {
            $_.SourceIdentifier -ceq $Record.SourceIdentifier
        })
        if ($FinalEvents.Count -ne 0) {
            $Observed = $true
        }
        foreach ($EventRecord in $FinalEvents) {
            Remove-Event -EventIdentifier $EventRecord.EventIdentifier -ErrorAction Stop
        }
    } catch {
        $TeardownFailed = $true
    }
    try {
        $Record.Watcher.Dispose()
    } catch {
        $TeardownFailed = $true
    }
    if ($Observed) {
        throw "a rejected installer started the desktop process"
    }
    if ($TeardownFailed) {
        throw "rejected-process observer teardown failed"
    }
}

function Invoke-RejectedInstaller([string[]] $Arguments, [int] $ExpectedExit, [string] $CaseId) {
    $Observer = $null
    try {
        $Observer = Start-RejectedDesktopProcessObserver
        Invoke-Installer $Arguments $ExpectedExit $CaseId
        Assert-RejectedDesktopProcessObserverClear $Observer
    } finally {
        if ($null -ne $Observer) {
            Stop-RejectedDesktopProcessObserver $Observer
        }
    }
}

function Wait-OwnedDesktopProcess([string] $ExpectedPath) {
    $script:CurrentCase = "updater-restart-process"
    for ($Attempt = 0; $Attempt -lt 100; $Attempt++) {
        $Matches = @(Get-DesktopProcessesAtExactPath $ExpectedPath)
        if ($Matches.Count -eq 1) {
            $Process = [System.Diagnostics.Process]::GetProcessById(
                [int]$Matches[0].ProcessId
            )
            try {
                $SafeHandle = $Process.SafeHandle
                if ($SafeHandle.IsInvalid -or $SafeHandle.IsClosed) {
                    throw "updater restart process handle is unavailable"
                }
                $ActualPath = [IO.Path]::GetFullPath($Process.MainModule.FileName)
                $ExpectedFullPath = [IO.Path]::GetFullPath($ExpectedPath)
                if (-not $ActualPath.Equals(
                    $ExpectedFullPath,
                    [StringComparison]::OrdinalIgnoreCase
                )) {
                    throw "updater restart process path changed during capture"
                }
                $Record = [pscustomobject]@{
                    Process = $Process
                    SafeHandle = $SafeHandle
                    ProcessId = $Process.Id
                    ExecutablePath = $ActualPath
                    StartTimeUtcTicks = $Process.StartTime.ToUniversalTime().Ticks
                }
                $script:OwnedDesktopProcess = $Record
                return $Record
            } catch {
                $Process.Dispose()
                throw "updater restart process identity capture failed"
            }
        }
        if ($Matches.Count -gt 1) {
            throw "the updater restart produced multiple exact-path processes"
        }
        Start-Sleep -Milliseconds 100
    }
    throw "the updater restart did not produce the exact-path process"
}

function Stop-OwnedDesktopProcess($Record) {
    $Process = $Record.Process
    try {
        if ($null -eq $Process -or
            $null -eq $Record.SafeHandle -or
            -not [object]::ReferenceEquals($Record.SafeHandle, $Process.SafeHandle) -or
            $Record.SafeHandle.IsInvalid -or
            $Record.SafeHandle.IsClosed -or
            $Record.ProcessId -ne $Process.Id) {
            throw "owned desktop process identity is invalid"
        }
        if ($Process.HasExited) { return }
        $ActualPath = [IO.Path]::GetFullPath($Process.MainModule.FileName)
        $ActualStartTimeUtcTicks = $Process.StartTime.ToUniversalTime().Ticks
        if (-not $ActualPath.Equals(
            $Record.ExecutablePath,
            [StringComparison]::OrdinalIgnoreCase
        ) -or $ActualStartTimeUtcTicks -ne $Record.StartTimeUtcTicks) {
            throw "owned desktop process identity changed"
        }
        $Process.Kill()
        if (-not $Process.WaitForExit($ProcessCleanupWaitMilliseconds)) {
            throw "owned desktop process did not exit within the bounded wait"
        }
    } finally {
        if ($null -ne $Process) {
            $Process.Dispose()
        }
        $Record.Process = $null
        $Record.SafeHandle = $null
    }
}

function Stop-RegisteredDesktopProcess([string] $ExpectedPath) {
    $ExpectedFullPath = [IO.Path]::GetFullPath($ExpectedPath)
    if ($null -ne $OwnedDesktopProcess) {
        $Record = $OwnedDesktopProcess
        try {
            if (-not $Record.ExecutablePath.Equals(
                $ExpectedFullPath,
                [StringComparison]::OrdinalIgnoreCase
            )) {
                throw "registered desktop process path does not match the owned install"
            }
            Stop-OwnedDesktopProcess $Record
        } finally {
            if ($null -ne $Record.Process) {
                $Record.Process.Dispose()
                $Record.Process = $null
                $Record.SafeHandle = $null
            }
        }
        $script:OwnedDesktopProcess = $null
    }
    if (@(Get-DesktopProcessesAtExactPath $ExpectedPath).Count -ne 0) {
        throw "an unregistered exact-path desktop process blocks cleanup"
    }
}

function Assert-CleanRejection([string[]] $Arguments, [string] $CaseId) {
    Assert-NoDesktopProcess $CaseId
    $WixBefore = @($OwnedRegistryFixtures | ForEach-Object {
        Get-RegistryFingerprint $_.Path
    })
    Invoke-RejectedInstaller $Arguments 2 $CaseId
    Assert-NoDesktopProcess $CaseId
    if ((Test-Path -LiteralPath $InstallDir) -or
        (Test-Path -LiteralPath $UninstallRegistryPath) -or
        (Test-Path -LiteralPath $ProductRegistryPath) -or
        @(Get-AirWikiShortcuts).Count -ne 0) {
        throw "a clean rejection mutated client state"
    }
    $WixAfter = @($OwnedRegistryFixtures | ForEach-Object {
        Get-RegistryFingerprint $_.Path
    })
    if (($WixAfter | ConvertTo-Json -Compress) -cne
        ($WixBefore | ConvertTo-Json -Compress)) {
        throw "a clean rejection changed a WiX fixture"
    }
}

function Assert-RejectedInstallerPreservedState(
    [string[]] $Arguments,
    [string] $CaseId
) {
    Assert-NoDesktopProcess $CaseId
    $Notice = Join-Path $InstallDir "THIRD_PARTY_NOTICES.md"
    $Sentinel = [Guid]::NewGuid().ToString("N")
    Assert-MutationProcessPrecondition $CaseId
    [IO.File]::WriteAllText($Notice, $Sentinel, [Text.UTF8Encoding]::new($false))
    Assert-MutationProcessPrecondition $CaseId
    Set-ItemProperty -LiteralPath $UninstallRegistryPath -Name DisplayIcon -Value $Sentinel
    $MetadataBefore = Get-RegistryFingerprint $UninstallRegistryPath
    $WixBefore = @($OwnedRegistryFixtures | ForEach-Object {
        Get-RegistryFingerprint $_.Path
    })
    Invoke-RejectedInstaller $Arguments 2 $CaseId
    Assert-NoDesktopProcess $CaseId
    if ([IO.File]::ReadAllText($Notice) -cne $Sentinel) {
        throw "a rejection changed the managed payload"
    }
    if ((Get-RegistryFingerprint $UninstallRegistryPath) -cne $MetadataBefore) {
        throw "a rejection changed NSIS metadata"
    }
    $WixAfter = @($OwnedRegistryFixtures | ForEach-Object {
        Get-RegistryFingerprint $_.Path
    })
    if (($WixAfter | ConvertTo-Json -Compress) -cne
        ($WixBefore | ConvertTo-Json -Compress)) {
        throw "a rejection changed WiX metadata"
    }
}

function Assert-SameFile([string] $Expected, [string] $Actual, [string] $Label) {
    if (-not (Test-Path -LiteralPath $Actual -PathType Leaf)) {
        throw "$Label is missing from the installation"
    }
    $ExpectedHash = (Get-FileHash -LiteralPath $Expected -Algorithm SHA256).Hash
    $ActualHash = (Get-FileHash -LiteralPath $Actual -Algorithm SHA256).Hash
    if ($ExpectedHash -ne $ActualHash) {
        throw "$Label differs from the verified release payload"
    }
}

function Get-AirWikiShortcuts {
    $Matches = @()
    if (Test-Path -LiteralPath $DesktopShortcut -PathType Leaf) {
        $Matches += $DesktopShortcut
    }
    if (Test-Path -LiteralPath $StartMenuRoot -PathType Container) {
        $Matches += @(Get-ChildItem -LiteralPath $StartMenuRoot -Recurse -File `
            -Filter "AirWiki.lnk" -ErrorAction Stop | ForEach-Object FullName)
    }
    return @($Matches)
}

function Get-ExistingAirWikiWixCount {
    $Count = 0
    foreach ($Key in @(Get-ChildItem -LiteralPath $WixUninstallRoot)) {
        if ([string]$Key.GetValue("DisplayName") -ceq "AirWiki" -and
            [string]$Key.GetValue("Publisher") -ceq $ExpectedPublisher) {
            $Count++
        }
    }
    return $Count
}

function Assert-OwnedCleanupLocation {
    Assert-NoReparsePath $ProgramDataRoot
    Assert-NoReparsePath $OwnerMarker
    if ([IO.File]::ReadAllText($OwnerMarker) -cne $OwnerToken) {
        throw "installer-matrix ownership marker changed"
    }
    $ExpectedParent = [IO.Path]::GetFullPath($ProgramDataRoot).TrimEnd(
        [char[]]@([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
    )
    $ActualParent = [IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($InstallDir))
    if (-not $ActualParent.Equals($ExpectedParent, [StringComparison]::OrdinalIgnoreCase) -or
        -not [IO.Path]::GetFileName($InstallDir).StartsWith(
            "airwiki-installer-gate-",
            [StringComparison]::Ordinal
        )) {
        throw "installer-matrix directory escaped the owned root"
    }
    if (Test-Path -LiteralPath $InstallDir) {
        Assert-NoReparsePath $InstallDir
    }
}

function Assert-InstalledRelease([string] $CaseId) {
    $script:CurrentCase = $CaseId
    Assert-SameFile $UninstallerReceipt $InstalledUninstaller "materialized uninstaller"
    Assert-WindowsPeMachine $InstalledUninstaller 0x014c "materialized NSIS uninstaller"
    $InstalledUninstallerSigner = Assert-ExpectedWindowsSigner $InstalledUninstaller
    Assert-SameWindowsSigner $ExpectedSigner $InstalledUninstallerSigner "materialized uninstaller"
    $script:UninstallerVerified = $true

    $UninstallMetadata = Get-ItemProperty -LiteralPath $UninstallRegistryPath
    if ([string]$UninstallMetadata.DisplayName -cne "AirWiki" -or
        [string]$UninstallMetadata.Publisher -cne $ExpectedPublisher -or
        [string]$UninstallMetadata.DisplayVersion -cne "0.2.0" -or
        [string]$UninstallMetadata.InstallLocation.Trim('"') -cne $InstallDir -or
        [string]$UninstallMetadata.UninstallString.Trim('"') -cne $InstalledUninstaller -or
        [string]$UninstallMetadata.DisplayIcon.Trim('"') -cne $DesktopExecutable) {
        throw "installer registered unexpected uninstall metadata"
    }
    $ProductKey = Get-Item -LiteralPath $ProductRegistryPath
    if ([string]$ProductKey.GetValue("") -cne $InstallDir) {
        throw "installer registered unexpected product metadata"
    }
    if (@(Get-AirWikiShortcuts).Count -ne 0) {
        throw "silent or passive installer ignored /NS"
    }

    Assert-SameFile (Join-Path $ReleaseDir "airwiki.exe") `
        $DesktopExecutable "desktop executable"
    Assert-SameFile (Join-Path $ReleaseDir "airwiki-mcp-bridge.exe") `
        (Join-Path $InstallDir "integrations\bridge\airwiki-mcp-bridge.exe") "MCP bridge"
    Assert-SameFile (Join-Path $ReleaseDir "airwiki-windows-firewall-helper.exe") `
        (Join-Path $InstallDir "airwiki-windows-firewall-helper.exe") "firewall helper"
    Assert-WindowsRuntimeTreeMatches $LlamaRuntime (Join-Path $InstallDir "llama")
    $null = Get-WindowsLlamaRuntimeManifest (Join-Path $InstallDir "llama") $LlamaPolicy
    Assert-WindowsDesktopEmbedsLlamaRuntimeHash `
        $DesktopExecutable `
        (Join-Path $InstallDir "llama") `
        $LlamaPolicy

    Assert-WindowsPeMachine $DesktopExecutable 0x8664 "installed desktop executable"
    Assert-WindowsPeMachine `
        (Join-Path $InstallDir "integrations\bridge\airwiki-mcp-bridge.exe") `
        0x8664 `
        "installed MCP bridge"
    Assert-WindowsPeMachine `
        (Join-Path $InstallDir "airwiki-windows-firewall-helper.exe") `
        0x8664 `
        "installed firewall helper"
    Assert-WindowsFirewallHelperManifest `
        (Join-Path $InstallDir "airwiki-windows-firewall-helper.exe") `
        "installed Windows firewall helper"

    $ExpectedPayload = New-WindowsPayloadManifest
    Add-WindowsPayloadFile $ExpectedPayload "airwiki.exe" `
        (Join-Path $ReleaseDir "airwiki.exe") "desktop executable"
    Add-WindowsPayloadFile $ExpectedPayload `
        "airwiki-windows-firewall-helper.exe" `
        (Join-Path $ReleaseDir "airwiki-windows-firewall-helper.exe") `
        "firewall helper"
    Add-WindowsPayloadFile $ExpectedPayload `
        "integrations/bridge/airwiki-mcp-bridge.exe" `
        (Join-Path $ReleaseDir "airwiki-mcp-bridge.exe") `
        "MCP bridge"
    Add-WindowsPayloadFile $ExpectedPayload `
        "integrations/airwiki-claude.mcpb" `
        $Mcpb `
        "Claude MCPB"
    Add-WindowsPayloadFile $ExpectedPayload "LICENSE" `
        (Join-Path $Root "LICENSE") "project license"
    Add-WindowsPayloadFile $ExpectedPayload "THIRD_PARTY_NOTICES.md" `
        (Join-Path $Root "THIRD_PARTY_NOTICES.md") "third-party notices"
    Add-WindowsPayloadTree $ExpectedPayload "licenses" `
        (Join-Path $Root "resources\licenses") "license inventory"
    Add-WindowsPayloadTree $ExpectedPayload "llama" $LlamaRuntime "llama.cpp runtime"
    Add-WindowsPayloadFile $ExpectedPayload "uninstall.exe" `
        $UninstallerReceipt "materialized uninstaller"
    Assert-WindowsPayloadMatches $ExpectedPayload $InstallDir "Windows installer matrix"
}

function Restore-VerifiedInstall([string] $CaseId) {
    $script:CurrentCase = $CaseId
    Set-InstalledVersionFixture "0.2.0"
    Invoke-Installer @("/S", "/NS", "/D=$InstallDir") 0 $CaseId
    Assert-InstalledRelease $CaseId
}

function Invoke-CleanupStep([string] $Label, [scriptblock] $Action) {
    $PreviousCase = $script:CurrentCase
    try {
        & $Action | Out-Null
        $script:CurrentCase = $PreviousCase
    } catch {
        $CleanupFailures.Add($Label) | Out-Null
        $script:CurrentCase = "cleanup-$Label"
    }
}

try {
    $CurrentCase = "host-preflight"
    $ClientOsVersion = Assert-WindowsClientInstallerGateHost

    $CurrentCase = "input-preflight"
    if ([string]::IsNullOrWhiteSpace($env:ProgramData)) {
        throw "the system-owned installer gate root is unavailable"
    }
    $ProgramDataRoot = (Resolve-Path -LiteralPath $env:ProgramData).Path
    $Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
    $ReleaseDir = Join-Path $Root "target\x86_64-pc-windows-msvc\release"
    $Mcpb = Join-Path $Root "target\mcpb\x86_64-pc-windows-msvc\airwiki-claude.mcpb"
    $OutDir = Join-Path $Root "target\packages\windows"
    $RunSuffix = [Guid]::NewGuid().ToString("N")
    $InstallDir = Join-Path $ProgramDataRoot "airwiki-installer-gate-$RunSuffix"
    $OwnerMarker = "$InstallDir.owner"
    $OwnerToken = [Guid]::NewGuid().ToString("N")
    $RegistryOwner = [Guid]::NewGuid().ToString("D")
    $WixUninstallRoot = "HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall"
    $ExpectedPublisher = "AirWiki"
    $UninstallerReceipt = Join-Path $Root "target\windows-uninstaller\airwiki-uninstall.exe"
    $LlamaRuntime = Join-Path $Root "resources\llama\windows-x64"
    $LlamaPolicy = Join-Path $Root "packaging\llama-windows-build-policy.json"
    $UninstallRegistryPath = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\AirWiki"
    $ProductRegistryPath = "HKCU:\Software\AirWiki\AirWiki"
    $DesktopExecutable = Join-Path $InstallDir "airwiki.exe"
    $InstalledUninstaller = Join-Path $InstallDir "uninstall.exe"
    $DesktopShortcut = Join-Path ([Environment]::GetFolderPath("Desktop")) "AirWiki.lnk"
    $StartMenuRoot = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs"

    . (Join-Path $PSScriptRoot "windows-signing.ps1")
    . (Join-Path $PSScriptRoot "windows-runtime.ps1")
    . (Join-Path $PSScriptRoot "windows-payload.ps1")
    . (Join-Path $PSScriptRoot "windows-safe-staging.ps1")

    Assert-NoReparsePath $ProgramDataRoot
    if (-not [IO.Path]::IsPathRooted($InstallDir) -or $InstallDir -match '\s') {
        throw "the installer gate path must be absolute and whitespace-free"
    }
    if ((Test-Path -LiteralPath $InstallDir) -or (Test-Path -LiteralPath $OwnerMarker)) {
        throw "the unique installer gate path already exists"
    }
    if ((Test-Path -LiteralPath $UninstallRegistryPath) -or
        (Test-Path -LiteralPath $ProductRegistryPath) -or
        @(Get-AirWikiShortcuts).Count -ne 0 -or
        (Get-ExistingAirWikiWixCount) -ne 0) {
        throw "the installer matrix requires a clean client"
    }
    Assert-NoDesktopProcess "input-preflight"

    $PackagerPolicy = [IO.File]::ReadAllText((Join-Path $Root "packaging\windows\Packager.toml"))
    if (@($PackagerPolicy -split "`r?`n" |
        Where-Object { $_ -ceq 'publisher = "AirWiki"' }).Count -ne 1) {
        throw "the fixture publisher does not match package policy"
    }
    $Installers = @(Get-ChildItem -LiteralPath $OutDir -File -Filter *.exe)
    if ($Installers.Count -ne 1) {
        throw "the installer matrix requires exactly one NSIS installer"
    }
    $Installer = $Installers[0].FullName
    $ExpectedSigner = Assert-ExpectedWindowsSigner $Installer
    Assert-WindowsPeMachine $Installer 0x014c "final NSIS installer"
    $InstallerHash = (Get-FileHash -LiteralPath $Installer -Algorithm SHA256).Hash.ToLowerInvariant()
    $null = Get-WindowsLlamaRuntimeManifest $LlamaRuntime $LlamaPolicy
    Assert-WindowsDesktopEmbedsLlamaRuntimeHash `
        (Join-Path $ReleaseDir "airwiki.exe") `
        $LlamaRuntime `
        $LlamaPolicy

    try {
        $CurrentCase = "owner-marker"
        Assert-MutationProcessPrecondition $CurrentCase
        [IO.File]::WriteAllText($OwnerMarker, $OwnerToken, [Text.UTF8Encoding]::new($false))
        $OwnerMarkerCreated = $true
        Assert-OwnedCleanupLocation

        Assert-CleanRejection @("/P", "/R", "/AIRWIKIUPDATE", "/NS", "/D=$InstallDir") "clean /AIRWIKIUPDATE rejection"

        $WixOne = $null
        try {
            $WixOne = New-OwnedWixFixture "one" "0.1.9"
            Assert-CleanRejection @("/S", "/NS", "/D=$InstallDir") "single WiX /S rejection"
            Assert-CleanRejection @("/P", "/NS", "/D=$InstallDir") "single WiX /P rejection"
            Assert-CleanRejection @("/P", "/R", "/AIRWIKIUPDATE", "/NS", "/D=$InstallDir") "single WiX /AIRWIKIUPDATE rejection"
            $WixTwo = $null
            try {
                $WixTwo = New-OwnedWixFixture "two" "0.1.8"
                Assert-CleanRejection @("/S", "/NS", "/D=$InstallDir") "multiple WiX rejection"
            } finally {
                if ($null -ne $WixTwo) { Remove-OwnedRegistryFixture $WixTwo }
            }
        } finally {
            if ($null -ne $WixOne) { Remove-OwnedRegistryFixture $WixOne }
        }

        Invoke-Installer @("/S", "/NS", "/D=$InstallDir") 0 "clean install"
        Assert-InstalledRelease "clean install verification"

        try {
            Assert-MutationProcessPrecondition $CurrentCase
            Remove-Item -LiteralPath (Join-Path $InstallDir "LICENSE") -Force
            Invoke-Installer @("/S", "/NS", "/D=$InstallDir") 0 "same-version silent repair"
            Assert-SameFile (Join-Path $Root "LICENSE") `
                (Join-Path $InstallDir "LICENSE") "repaired license"
            Assert-InstalledRelease "same-version silent repair verification"
        } finally {
            Restore-VerifiedInstall "same-version silent repair recovery"
        }

        try {
            Set-InstalledVersionFixture "0.2.0"
            Assert-RejectedInstallerPreservedState `
                @("/P", "/R", "/AIRWIKIUPDATE", "/NS", "/D=$InstallDir") `
                "/AIRWIKIUPDATE rejects same-version replay"
        } finally {
            Restore-VerifiedInstall "/AIRWIKIUPDATE same-version replay recovery"
        }

        try {
            Set-InstalledVersionFixture "0.1.9"
            Assert-RejectedInstallerPreservedState `
                @("/R", "/AIRWIKIUPDATE", "/NS", "/D=$InstallDir") `
                "/AIRWIKIUPDATE requires /P"
        } finally {
            Restore-VerifiedInstall "/AIRWIKIUPDATE requires /P recovery"
        }

        try {
            Set-InstalledVersionFixture "0.1.9"
            Assert-MutationProcessPrecondition $CurrentCase
            Remove-Item -LiteralPath (Join-Path $InstallDir "LICENSE") -Force
            Invoke-Installer @("/S", "/NS", "/D=$InstallDir") 0 "strictly newer silent upgrade"
            Assert-InstalledRelease "strictly newer silent upgrade verification"
        } finally {
            Restore-VerifiedInstall "strictly newer silent upgrade recovery"
        }

        try {
            Set-InstalledVersionFixture "0.2.1"
            Assert-RejectedInstallerPreservedState `
                @("/S", "/NS", "/D=$InstallDir") `
                "silent downgrade rejection"
        } finally {
            Restore-VerifiedInstall "silent downgrade rejection recovery"
        }

        $CoexistingWix = $null
        try {
            $CoexistingWix = New-OwnedWixFixture "coexist" "0.1.9"
            Assert-RejectedInstallerPreservedState `
                @("/S", "/NS", "/D=$InstallDir") `
                "WiX plus NSIS coexistence rejection"
        } finally {
            if ($null -ne $CoexistingWix) { Remove-OwnedRegistryFixture $CoexistingWix }
            Restore-VerifiedInstall "WiX plus NSIS coexistence rejection recovery"
        }

        foreach ($RequiredName in @(
            "DisplayName",
            "Publisher",
            "InstallLocation",
            "UninstallString",
            "DisplayVersion"
        )) {
            $RequiredValue = (Get-ItemProperty -LiteralPath $UninstallRegistryPath).$RequiredName
            try {
                Assert-MutationProcessPrecondition $CurrentCase
                Remove-ItemProperty -LiteralPath $UninstallRegistryPath -Name $RequiredName
                Assert-RejectedInstallerPreservedState `
                    @("/S", "/NS", "/D=$InstallDir") `
                    "partial NSIS key rejection: $RequiredName"
            } finally {
                Assert-MutationProcessPrecondition $CurrentCase
                Set-ItemProperty -LiteralPath $UninstallRegistryPath -Name $RequiredName -Value $RequiredValue
                Restore-VerifiedInstall "partial NSIS key rejection recovery: $RequiredName"
            }
        }

        try {
            Set-InstalledVersionFixture "not-semver"
            Assert-RejectedInstallerPreservedState `
                @("/S", "/NS", "/D=$InstallDir") `
                "invalid installed version rejection"
        } finally {
            Restore-VerifiedInstall "invalid installed version rejection recovery"
        }

        Assert-NoDesktopProcess "/AIRWIKIUPDATE strictly newer install"
        $UpdaterRestartCleanupAllowed = $false
        try {
            Set-InstalledVersionFixture "0.1.9"
            Invoke-Installer `
                @("/P", "/R", "/AIRWIKIUPDATE", "/NS", "/D=$InstallDir") `
                0 `
                "/AIRWIKIUPDATE strictly newer install"
            $OwnedDesktopProcess = Wait-OwnedDesktopProcess $DesktopExecutable
            Assert-InstalledRelease "/AIRWIKIUPDATE strictly newer install verification"
        } finally {
            Invoke-CleanupStep "updater-restart-process" {
                Stop-RegisteredDesktopProcess $DesktopExecutable
                $script:UpdaterRestartCleanupAllowed = $true
            }
            if ($UpdaterRestartCleanupAllowed) {
                Invoke-CleanupStep "updater-install-recovery" {
                    Restore-VerifiedInstall "/AIRWIKIUPDATE strictly newer install recovery"
                }
            }
        }

        Assert-InstalledRelease "final installed release verification"
        Assert-NoDesktopProcess "signed uninstaller"
        $CurrentCase = "signed uninstaller"
        Assert-MutationProcessPrecondition $CurrentCase
        $UninstallProcess = Start-Process `
            -FilePath $InstalledUninstaller `
            -ArgumentList "/S" `
            -Wait `
            -PassThru
        if ($UninstallProcess.ExitCode -ne 0) {
            throw "silent uninstaller returned a nonzero exit"
        }
        for ($Attempt = 0; $Attempt -lt 100 -and
            (Test-Path -LiteralPath $InstallDir); $Attempt++) {
            Start-Sleep -Milliseconds 100
        }
        if ((Test-Path -LiteralPath $InstallDir) -or
            (Test-Path -LiteralPath $UninstallRegistryPath) -or
            (Test-Path -LiteralPath $ProductRegistryPath) -or
            @(Get-AirWikiShortcuts).Count -ne 0) {
            throw "silent uninstaller left managed state"
        }
        $UninstallerPassed = $true
    } finally {
        Invoke-CleanupStep "owned-desktop-process" {
            Stop-RegisteredDesktopProcess $DesktopExecutable
        }
        foreach ($Record in @($OwnedRegistryFixtures)) {
            Invoke-CleanupStep "owned registry fixture" { Remove-OwnedRegistryFixture $Record }
        }
        Invoke-CleanupStep "desktop-process-check" {
            if ($null -ne $OwnedDesktopProcess -or
                @(Get-DesktopProcessesAtExactPath $DesktopExecutable).Count -ne 0) {
                throw "install cleanup requires exact process exit"
            }
            $script:InstallCleanupAllowed = $true
        }
        if ($InstallCleanupAllowed -and
            $UninstallerVerified -and
            (Test-Path -LiteralPath $InstalledUninstaller -PathType Leaf)) {
            Invoke-CleanupStep "signed-uninstaller" {
                Assert-SameFile $UninstallerReceipt $InstalledUninstaller "materialized uninstaller"
                $CleanupSigner = Assert-ExpectedWindowsSigner $InstalledUninstaller
                Assert-SameWindowsSigner $ExpectedSigner $CleanupSigner "materialized uninstaller"
                Assert-MutationProcessPrecondition $CurrentCase
                $CleanupProcess = Start-Process `
                    -FilePath $InstalledUninstaller `
                    -ArgumentList "/S" `
                    -Wait `
                    -PassThru
                if ($CleanupProcess.ExitCode -ne 0) { throw "cleanup uninstaller failed" }
            }
        }
        if ($InstallCleanupAllowed -and $OwnerMarkerCreated) {
            Invoke-CleanupStep "install-staging" {
                Assert-OwnedCleanupLocation
                Assert-MutationProcessPrecondition $CurrentCase
                Remove-AirWikiWindowsStagingPath `
                    -Path $InstallDir `
                    -AllowedRoot $ProgramDataRoot `
                    -Label "Windows installer matrix staging"
            }
            Invoke-CleanupStep "owner-marker" {
                Assert-OwnedCleanupLocation
                Assert-MutationProcessPrecondition $CurrentCase
                Remove-AirWikiWindowsStagingPath `
                    -Path $OwnerMarker `
                    -AllowedRoot $ProgramDataRoot `
                    -Label "Windows installer matrix owner marker"
            }
        }
        if ($OwnedRegistryFixtures.Count -ne 0) {
            $CleanupFailures.Add("owned-registry-residue") | Out-Null
        }
        if ($CleanupFailures.Count -ne 0) {
            $CurrentCase = "cleanup"
            throw "one or more cleanup categories failed"
        }
    }

    if (-not $UninstallerPassed -or $OwnedRegistryFixtures.Count -ne 0) {
        throw "final installer matrix state is incomplete"
    }
    [Console]::Out.WriteLine(
        "WINDOWS_INSTALLER_MATRIX_PASS os_version=$ClientOsVersion installer_sha256=$InstallerHash transition_matrix=pass uninstaller=pass owned_residue=0"
    )
} catch {
    [Console]::Error.WriteLine("WINDOWS_INSTALLER_MATRIX_FAIL case=$CurrentCase")
    exit 1
}
