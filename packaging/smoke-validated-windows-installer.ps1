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
$ModelReadyWaitMilliseconds = 1800000
$McpRequestTimeoutSeconds = 8
$McpRequestId = 991
$ProcessWaitMilliseconds = 120000
$ProcessCleanupWaitMilliseconds = 10000
$ActivationStatusSettleWaitMilliseconds = 30000
$ActivationLogPollMilliseconds = 100
$ActivationLogReadLimitBytes = 1048576
$ActivationStatusReadLimitBytes = 4096
$ModelActivationErrorKinds = @(
    "configuration",
    "onnx_init",
    "embedding_smoke",
    "reranker_smoke",
    "runtime_spawn",
    "runtime_exit_before_health",
    "runtime_health_timeout",
    "runtime_state",
    "generation_timeout",
    "generation_unavailable",
    "generation_protocol",
    "generation_invalid",
    "activation_internal",
    "install_network",
    "install_integrity",
    "install_storage",
    "install_promotion",
    "install_runtime_verification",
    "install_capacity",
    "install_configuration",
    "install_cancelled",
    "install_internal"
)
$ModelInstallErrorKinds = @(
    "install_network",
    "install_integrity",
    "install_storage",
    "install_promotion",
    "install_runtime_verification",
    "install_capacity",
    "install_configuration",
    "install_cancelled",
    "install_internal"
)
$ModelActivationElapsedBuckets = @(
    "under_5s",
    "5s_to_30s",
    "30s_to_120s",
    "120s_to_300s",
    "over_300s"
)
$ModelActivationExitClasses = @(
    "none",
    "success",
    "failure",
    "unknown"
)
$ModelActivationStates = @(
    "starting",
    "ready",
    "failed"
)
$InstallerSmokeStages = @(
    "preflight",
    "installer",
    "registration",
    "desktop_correlation",
    "payload_validation",
    "models",
    "uninstall_cleanup",
    "complete",
    "unknown"
)
$InstallerSmokeFailureClasses = @(
    "payload_validation_failed",
    "model_activation_failed",
    "model_install_failed",
    "desktop_exited_before_ready",
    "runtime_exited_before_ready",
    "models_timeout",
    "powershell_runtime"
)
$PayloadDifferenceClasses = @(
    "set_mismatch",
    "missing_directory",
    "missing_file",
    "bytes_mismatch"
)
$AvailableMemoryBuckets = @(
    "unknown",
    "under_2gib",
    "2gib_to_4gib",
    "over_4gib"
)
$InstallerSmokeCleanupStatuses = @(
    "not_needed",
    "pass",
    "failed",
    "unknown"
)
$script:ProcessTerminationUnconfirmed = $false
$script:TerminalStage = "preflight"
$script:StructuredFailure = $null
$script:PayloadValidationDiagnostic = $null
$ExpectedDesktopProcess = $null

function Set-StructuredInstallerSmokeFailure(
    [string] $FailureClass,
    $ErrorKind,
    $ElapsedBucket,
    [string] $ExitClass
) {
    $SafeClass = $FailureClass
    $SafeErrorKind = $null
    $SafeElapsedBucket = $null
    $SafeExitClass = $ExitClass

    if ($InstallerSmokeFailureClasses -cnotcontains $SafeClass -or
        $ModelActivationExitClasses -cnotcontains $SafeExitClass) {
        $SafeClass = "powershell_runtime"
        $SafeExitClass = "failure"
    } elseif ($SafeClass -eq "model_activation_failed" -or
        $SafeClass -eq "model_install_failed") {
        if ($ModelActivationErrorKinds -ccontains $ErrorKind -and
            $ModelActivationElapsedBuckets -ccontains $ElapsedBucket) {
            $SafeErrorKind = $ErrorKind
            $SafeElapsedBucket = $ElapsedBucket
            $IsInstallKind = $ModelInstallErrorKinds -ccontains $ErrorKind
            if (($SafeClass -eq "model_install_failed") -ne $IsInstallKind) {
                $SafeClass = "powershell_runtime"
                $SafeErrorKind = $null
                $SafeElapsedBucket = $null
                $SafeExitClass = "failure"
            }
        } else {
            $SafeClass = "powershell_runtime"
            $SafeExitClass = "failure"
        }
    }

    $script:StructuredFailure = [PSCustomObject]@{
        FailureClass = $SafeClass
        ErrorKind = $SafeErrorKind
        ElapsedBucket = $SafeElapsedBucket
        ExitClass = $SafeExitClass
    }
}

function Set-PayloadValidationFailure($Expected, $Actual) {
    $DifferenceClass = $null
    if ($Expected.Files.Count -ne $Actual.Files.Count -or
        $Expected.Directories.Count -ne $Actual.Directories.Count) {
        $DifferenceClass = "set_mismatch"
    } else {
        foreach ($Relative in $Expected.Directories.Keys) {
            if (-not $Actual.Directories.ContainsKey($Relative)) {
                $DifferenceClass = "missing_directory"
                break
            }
        }
        if ($null -eq $DifferenceClass) {
            foreach ($Relative in $Expected.Files.Keys) {
                if (-not $Actual.Files.ContainsKey($Relative)) {
                    $DifferenceClass = "missing_file"
                    break
                }
                if ($Expected.Files[$Relative].Length -ne $Actual.Files[$Relative].Length -or
                    $Expected.Files[$Relative].Sha256 -ne $Actual.Files[$Relative].Sha256) {
                    $DifferenceClass = "bytes_mismatch"
                    break
                }
            }
        }
    }
    if ($null -eq $DifferenceClass) {
        return
    }
    Set-StructuredInstallerSmokeFailure `
        "payload_validation_failed" `
        $null `
        $null `
        "failure"
    $script:PayloadValidationDiagnostic = [PSCustomObject]@{
        DifferenceClass = $DifferenceClass
        ExpectedFiles = [int] $Expected.Files.Count
        ActualFiles = [int] $Actual.Files.Count
        ExpectedDirectories = [int] $Expected.Directories.Count
        ActualDirectories = [int] $Actual.Directories.Count
    }
}

function Invoke-Process(
    [string] $Path,
    [string[]] $Arguments,
    [string] $Label
) {
    $Process = Start-Process -FilePath $Path -ArgumentList $Arguments -PassThru
    try {
        if (-not $Process.WaitForExit($ProcessWaitMilliseconds)) {
            $script:ProcessTerminationUnconfirmed = $true
            $Process.Kill()
            if (-not $Process.WaitForExit($ProcessCleanupWaitMilliseconds)) {
                throw "$Label termination was not confirmed after timeout cleanup"
            }
            $script:ProcessTerminationUnconfirmed = $false
            throw "$Label did not exit within the bounded wait"
        }
        $ExitCode = $Process.ExitCode
    } finally {
        $Process.Dispose()
    }
    if ($ExitCode -ne 0) {
        throw "$Label returned a nonzero exit code"
    }
}

function Get-DesktopProcesses {
    return @(Get-CimInstance Win32_Process -Filter "Name = 'airwiki.exe'")
}

function Invoke-AutomaticCleanup {
    if ($script:ProcessTerminationUnconfirmed) {
        throw "automatic cleanup was skipped because process termination was not confirmed; recover the partial per-user installation manually"
    }
    Remove-ExactRegisteredInstall
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

function Get-ExactModelRuntimeProcess(
    [int] $DesktopProcessId,
    [string] $ExpectedExecutable
) {
    $Matches = @(
        Get-CimInstance Win32_Process `
            -Filter "ParentProcessId = $DesktopProcessId AND Name = 'llama-server.exe'"
    )
    if ($Matches.Count -eq 0) {
        return $null
    }
    if ($Matches.Count -ne 1) {
        throw "the installed model runtime process state is ambiguous"
    }
    if (-not (Test-SamePath ([string] $Matches[0].ExecutablePath) $ExpectedExecutable)) {
        throw "the installed model runtime process identity changed"
    }
    return $Matches[0]
}

function Test-ModelRuntimeExitedBeforeReady(
    [int] $DesktopProcessId,
    [string] $ExpectedExecutable,
    [ref] $ObservedProcessId
) {
    $RuntimeProcess = Get-ExactModelRuntimeProcess `
        $DesktopProcessId `
        $ExpectedExecutable
    if ($null -eq $RuntimeProcess) {
        return $null -ne $ObservedProcessId.Value
    }

    $CurrentProcessId = [int] $RuntimeProcess.ProcessId
    if ($null -eq $ObservedProcessId.Value) {
        $ObservedProcessId.Value = $CurrentProcessId
        return $false
    }
    return [int] $ObservedProcessId.Value -ne $CurrentProcessId
}

function Get-RegularActivationStatusItem([string] $Path) {
    if (-not (Test-Path -LiteralPath $Path)) {
        return $null
    }
    $Item = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
    if ($Item.PSIsContainer -or
        (($Item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) -or
        [long] $Item.Length -le 0 -or
        [long] $Item.Length -gt $ActivationStatusReadLimitBytes) {
        throw "the local activation status is not a bounded regular file"
    }
    return $Item
}

function New-ActivationStatusCursor([string] $Path) {
    $Item = Get-RegularActivationStatusItem $Path
    return [PSCustomObject]@{
        Path = $Path
        BaselineLength = if ($null -eq $Item) { [long] -1 } else { [long] $Item.Length }
        BaselineWriteTicks = if ($null -eq $Item) {
            [long] -1
        } else {
            [long] $Item.LastWriteTimeUtc.Ticks
        }
        ObservedStarting = $false
    }
}

function Get-SanitizedModelActivationStatus($Cursor) {
    $Item = Get-RegularActivationStatusItem ([string] $Cursor.Path)
    if ($null -eq $Item) {
        return $null
    }
    $Changed = [long] $Item.Length -ne [long] $Cursor.BaselineLength -or
        [long] $Item.LastWriteTimeUtc.Ticks -ne [long] $Cursor.BaselineWriteTicks
    if (-not $Changed -and -not $Cursor.ObservedStarting) {
        return $null
    }

    $Stream = [IO.File]::Open(
        $Item.FullName,
        [IO.FileMode]::Open,
        [IO.FileAccess]::Read,
        [IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete
    )
    try {
        if ($Stream.Length -le 0 -or
            $Stream.Length -gt $ActivationStatusReadLimitBytes) {
            throw "the local activation status exceeded its bounded schema"
        }
        $Bytes = New-Object byte[] $Stream.Length
        $Read = 0
        while ($Read -lt $Bytes.Length) {
            $Count = $Stream.Read($Bytes, $Read, $Bytes.Length - $Read)
            if ($Count -eq 0) {
                break
            }
            $Read += $Count
        }
    } finally {
        $Stream.Dispose()
    }
    if ($Read -ne $Bytes.Length) {
        throw "the local activation status could not be read atomically"
    }

    $Utf8 = New-Object Text.UTF8Encoding($false, $true)
    $Record = $Utf8.GetString($Bytes).Trim() |
        ConvertFrom-Json -ErrorAction Stop
    $ExpectedProperties = @(
        "elapsed_bucket",
        "error_kind",
        "exit_class",
        "schema_version",
        "state"
    )
    $ActualProperties = @(
        $Record.PSObject.Properties.Name | Sort-Object
    )
    if ([string]::Join("|", $ActualProperties) -cne
        [string]::Join("|", $ExpectedProperties) -or
        ($Record.schema_version -isnot [int] -and
            $Record.schema_version -isnot [long]) -or
        [int] $Record.schema_version -ne 1) {
        throw "the local activation status schema is invalid"
    }

    $State = [string] $Record.state
    if ($ModelActivationStates -cnotcontains $State) {
        throw "the local activation status state is invalid"
    }
    if ($State -eq "starting") {
        if ($Changed) {
            $Cursor.ObservedStarting = $true
        }
        if ($null -ne $Record.error_kind -or
            $null -ne $Record.elapsed_bucket -or
            $null -ne $Record.exit_class) {
            throw "the local activation starting record is invalid"
        }
        return $null
    }
    if ($State -eq "ready") {
        if ($null -ne $Record.error_kind -or
            $null -ne $Record.elapsed_bucket -or
            $null -ne $Record.exit_class) {
            throw "the local activation ready record is invalid"
        }
        return [PSCustomObject]@{ State = "ready" }
    }

    $ErrorKind = [string] $Record.error_kind
    $ElapsedBucket = [string] $Record.elapsed_bucket
    $ExitClass = [string] $Record.exit_class
    if ($ModelActivationErrorKinds -cnotcontains $ErrorKind -or
        $ModelActivationElapsedBuckets -cnotcontains $ElapsedBucket -or
        $ModelActivationExitClasses -cnotcontains $ExitClass) {
        throw "the local activation failure record is invalid"
    }
    return [PSCustomObject]@{
        State = "failed"
        ErrorKind = $ErrorKind
        ElapsedBucket = $ElapsedBucket
        ExitClass = $ExitClass
    }
}

function Assert-RegularActivationLogDirectory([string] $Directory) {
    if (-not (Test-Path -LiteralPath $Directory)) {
        return $false
    }
    $Item = Get-Item -LiteralPath $Directory -Force -ErrorAction Stop
    if (-not $Item.PSIsContainer -or
        (($Item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0)) {
        throw "the local activation log directory is not a regular directory"
    }
    return $true
}

function Get-ActivationLogFiles([string] $Directory) {
    if (-not (Assert-RegularActivationLogDirectory $Directory)) {
        return @()
    }
    $Files = @(
        Get-ChildItem -LiteralPath $Directory -File -Force -ErrorAction Stop |
            Where-Object Name -Match '^airwiki[.]log([.][0-9]{4}-[0-9]{2}-[0-9]{2})?$' |
            Sort-Object Name
    )
    foreach ($File in $Files) {
        if (($File.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "the local activation log is not a regular file"
        }
    }
    return $Files
}

function New-ActivationLogCursor([string] $Directory) {
    $Offsets = @{}
    $Buffers = @{}
    foreach ($File in @(Get-ActivationLogFiles $Directory)) {
        $Offsets[$File.FullName] = [long] $File.Length
        $Buffers[$File.FullName] = ""
    }
    return [PSCustomObject]@{
        Directory = $Directory
        Offsets = $Offsets
        Buffers = $Buffers
        TotalBytes = [long] 0
    }
}

function Update-ActivationLogCursor($Cursor) {
    foreach ($File in @(Get-ActivationLogFiles ([string] $Cursor.Directory))) {
        $Path = $File.FullName
        $Offset = if ($Cursor.Offsets.ContainsKey($Path)) {
            [long] $Cursor.Offsets[$Path]
        } else {
            [long] 0
        }
        if ([long] $File.Length -lt $Offset) {
            throw "the local activation log changed unexpectedly"
        }

        $Length = [long] $File.Length - $Offset
        if ($Length -eq 0) {
            continue
        }
        if ($Cursor.TotalBytes + $Length -gt $ActivationLogReadLimitBytes) {
            throw "the bounded local activation log window was exceeded"
        }

        $Stream = [IO.File]::Open(
            $Path,
            [IO.FileMode]::Open,
            [IO.FileAccess]::Read,
            [IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete
        )
        try {
            $null = $Stream.Seek($Offset, [IO.SeekOrigin]::Begin)
            $Bytes = New-Object byte[] $Length
            $Read = 0
            while ($Read -lt $Length) {
                $Count = $Stream.Read($Bytes, $Read, $Length - $Read)
                if ($Count -eq 0) {
                    break
                }
                $Read += $Count
            }
        } finally {
            $Stream.Dispose()
        }
        if ($Read -eq 0) {
            continue
        }

        $Text = [Text.Encoding]::UTF8.GetString($Bytes, 0, $Read)
        $Existing = if ($Cursor.Buffers.ContainsKey($Path)) {
            [string] $Cursor.Buffers[$Path]
        } else {
            ""
        }
        $Cursor.Buffers[$Path] = $Existing + $Text
        $Cursor.Offsets[$Path] = $Offset + $Read
        $Cursor.TotalBytes += $Read
    }
}

function Get-SanitizedModelActivationFailure($Cursor) {
    # The tracing formatter does not make field order part of its contract.
    # Match each closed field independently and never return the source line.
    $Failure = $null
    foreach ($Text in @($Cursor.Buffers.Values)) {
        foreach ($Line in @([string] $Text -split "`r?`n")) {
            if (-not $Line.Contains("model activation failed") -or
                $Line -notmatch '\bevent="model_activation_failed"') {
                continue
            }

            $ErrorKindMatch = [regex]::Match(
                $Line,
                '\berror_kind="([a-z0-9_]+)"'
            )
            $ElapsedBucketMatch = [regex]::Match(
                $Line,
                '\belapsed_bucket="([a-z0-9_]+)"'
            )
            $ExitClassMatch = [regex]::Match(
                $Line,
                '\bexit_class="([a-z0-9_]+)"'
            )
            if (-not $ErrorKindMatch.Success -or
                -not $ElapsedBucketMatch.Success -or
                -not $ExitClassMatch.Success) {
                continue
            }

            $ErrorKind = $ErrorKindMatch.Groups[1].Value
            $ElapsedBucket = $ElapsedBucketMatch.Groups[1].Value
            $ExitClass = $ExitClassMatch.Groups[1].Value
            if ($ModelActivationErrorKinds -cnotcontains $ErrorKind -or
                $ModelActivationElapsedBuckets -cnotcontains $ElapsedBucket -or
                $ModelActivationExitClasses -cnotcontains $ExitClass) {
                continue
            }
            $Failure = [PSCustomObject]@{
                ErrorKind = $ErrorKind
                ElapsedBucket = $ElapsedBucket
                ExitClass = $ExitClass
            }
        }
    }
    return $Failure
}

function Throw-SanitizedModelActivationFailure($Failure) {
    $FailureClass = if ($ModelInstallErrorKinds -ccontains $Failure.ErrorKind) {
        "model_install_failed"
    } else {
        "model_activation_failed"
    }
    Set-StructuredInstallerSmokeFailure `
        $FailureClass `
        $Failure.ErrorKind `
        $Failure.ElapsedBucket `
        $Failure.ExitClass
    throw "the installed models did not become ready"
}

function Throw-IfModelActivationFailed {
    $Status = Get-SanitizedModelActivationStatus $ActivationStatusCursor
    if ($null -ne $Status -and $Status.State -eq "failed") {
        Throw-SanitizedModelActivationFailure $Status
    }
    if (-not $ActivationStatusCursor.ObservedStarting -and
        $null -eq $Status) {
        Update-ActivationLogCursor $ActivationLogCursor
        $Failure = Get-SanitizedModelActivationFailure $ActivationLogCursor
        if ($null -ne $Failure) {
            Throw-SanitizedModelActivationFailure $Failure
        }
    }
}

function Throw-IfDesktopExitedBeforeReady {
    if ($null -eq $ExpectedDesktopProcess -or
        -not $ExpectedDesktopProcess.HasExited) {
        return
    }
    $ExitClass = if ($ExpectedDesktopProcess.ExitCode -eq 0) {
        "success"
    } else {
        "failure"
    }
    Set-StructuredInstallerSmokeFailure `
        "desktop_exited_before_ready" `
        $null `
        $null `
        $ExitClass
    throw "the installed desktop exited before models became ready"
}

function Throw-ModelRuntimeExitedBeforeReady {
    $Deadline = [DateTime]::UtcNow.AddMilliseconds(
        $ActivationStatusSettleWaitMilliseconds
    )
    do {
        Throw-IfModelActivationFailed
        Throw-IfDesktopExitedBeforeReady
        if ([DateTime]::UtcNow -ge $Deadline) {
            break
        }
        Start-Sleep -Milliseconds $ActivationLogPollMilliseconds
    } while ($true)
    Set-StructuredInstallerSmokeFailure `
        "runtime_exited_before_ready" `
        $null `
        $null `
        "unknown"
    throw "the installed model runtime exited before models became ready"
}

function Assert-NoForeignDesktopProcess([string] $ExpectedExecutable) {
    foreach ($Process in @(Get-DesktopProcesses)) {
        if (-not (Test-SamePath ([string] $Process.ExecutablePath) $ExpectedExecutable)) {
            throw "another AirWiki executable is running; close it before this smoke test"
        }
    }
}

function Stop-ExactDesktopProcess([string] $ExpectedExecutable) {
    Assert-NoForeignDesktopProcess $ExpectedExecutable
    $Matches = @(Get-DesktopProcesses)
    if ($Matches.Count -eq 0) {
        return
    }
    if ($Matches.Count -ne 1) {
        throw "the installed desktop process state is ambiguous"
    }

    $Process = [Diagnostics.Process]::GetProcessById([int] $Matches[0].ProcessId)
    try {
        $SafeHandle = $Process.SafeHandle
        if ($SafeHandle.IsInvalid -or $SafeHandle.IsClosed -or $Process.HasExited) {
            throw "the installed desktop process identity is unavailable"
        }
        if (-not (Test-SamePath ([string] $Process.MainModule.FileName) $ExpectedExecutable)) {
            throw "the installed desktop process identity changed"
        }
        $Process.Kill()
        if (-not $Process.WaitForExit($ProcessCleanupWaitMilliseconds)) {
            throw "the installed desktop process did not exit within the bounded wait"
        }
    } finally {
        $Process.Dispose()
    }
    if ((@(Get-DesktopProcesses)).Count -ne 0) {
        throw "the installed desktop process did not remain stopped"
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

function Wait-ForExactRegisteredUninstaller {
    $Deadline = [DateTime]::UtcNow.AddMilliseconds($StateWaitMilliseconds)
    do {
        try {
            return (Get-ExactRegisteredUninstaller)
        } catch {
            Start-Sleep -Milliseconds 250
        }
    } while ([DateTime]::UtcNow -lt $Deadline)
    throw "the per-user installation did not become complete and coherently registered"
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
    Stop-ExactDesktopProcess $DesktopExecutable
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
    $ObservedRuntimeProcessId = $null
    $Deadline = [DateTime]::UtcNow.AddMilliseconds($ModelReadyWaitMilliseconds)
    do {
        Throw-IfModelActivationFailed
        Throw-IfDesktopExitedBeforeReady
        if (Test-ModelRuntimeExitedBeforeReady `
            $ExpectedDesktopProcessId `
            $ExpectedRuntimeExecutable `
            ([ref] $ObservedRuntimeProcessId)) {
            Throw-ModelRuntimeExitedBeforeReady
        }
        $ResponseLines = @($Body | & $Curl `
            --silent `
            --show-error `
            --noproxy "*" `
            --connect-timeout 2 `
            --max-time $McpRequestTimeoutSeconds `
            --header "Connection: close" `
            --header "Content-Type: application/json" `
            --header "Accept: application/json, text/event-stream" `
            --data-binary "@-" `
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
        Throw-IfModelActivationFailed
        Throw-IfDesktopExitedBeforeReady
        if (Test-ModelRuntimeExitedBeforeReady `
            $ExpectedDesktopProcessId `
            $ExpectedRuntimeExecutable `
            ([ref] $ObservedRuntimeProcessId)) {
            Throw-ModelRuntimeExitedBeforeReady
        }
        Start-Sleep -Seconds 3
    } while ([DateTime]::UtcNow -lt $Deadline)
    Set-StructuredInstallerSmokeFailure `
        "models_timeout" `
        $null `
        $null `
        "failure"
    throw "the installed application did not make its local models operational in time"
}

$InstalledByThisRun = $false
$PrimaryFailed = $false
$CleanupStatus = "not_needed"
$AvailableMemoryBucket = "unknown"
$InstallerHash = ""

function Get-SanitizedInstallerSmokeFailure([string] $CurrentCleanupStatus) {
    $Stage = [string] $script:TerminalStage
    if ($InstallerSmokeStages -cnotcontains $Stage) {
        $Stage = "unknown"
    }
    $MemoryBucket = [string] $AvailableMemoryBucket
    if ($AvailableMemoryBuckets -cnotcontains $MemoryBucket) {
        $MemoryBucket = "unknown"
    }
    if ($InstallerSmokeCleanupStatuses -cnotcontains $CurrentCleanupStatus) {
        $CurrentCleanupStatus = "unknown"
    }

    $FailureClass = "powershell_runtime"
    $ErrorKind = $null
    $ElapsedBucket = $null
    $ExitClass = "failure"
    $PayloadDifferenceClass = $null
    $ExpectedFileCount = $null
    $ActualFileCount = $null
    $ExpectedDirectoryCount = $null
    $ActualDirectoryCount = $null

    if ($null -ne $script:StructuredFailure) {
        $CandidateClass = [string] $script:StructuredFailure.FailureClass
        $CandidateExitClass = [string] $script:StructuredFailure.ExitClass
        if ($InstallerSmokeFailureClasses -ccontains $CandidateClass -and
            $ModelActivationExitClasses -ccontains $CandidateExitClass) {
            $FailureClass = $CandidateClass
            $ExitClass = $CandidateExitClass
        }

        if ($FailureClass -eq "model_activation_failed" -or
            $FailureClass -eq "model_install_failed") {
            $CandidateErrorKind = [string] $script:StructuredFailure.ErrorKind
            $CandidateElapsedBucket = `
                [string] $script:StructuredFailure.ElapsedBucket
            if ($ModelActivationErrorKinds -ccontains $CandidateErrorKind -and
                $ModelActivationElapsedBuckets -ccontains
                    $CandidateElapsedBucket) {
                $ErrorKind = $CandidateErrorKind
                $ElapsedBucket = $CandidateElapsedBucket
                $IsInstallKind = `
                    $ModelInstallErrorKinds -ccontains $CandidateErrorKind
                if (($FailureClass -eq "model_install_failed") -ne
                    $IsInstallKind) {
                    $FailureClass = "powershell_runtime"
                    $ErrorKind = $null
                    $ElapsedBucket = $null
                    $ExitClass = "failure"
                }
            } else {
                $FailureClass = "powershell_runtime"
                $ExitClass = "failure"
            }
        }
    }

    if ($FailureClass -eq "payload_validation_failed" -and
        $null -ne $script:PayloadValidationDiagnostic) {
        $CandidateDifferenceClass = `
            [string] $script:PayloadValidationDiagnostic.DifferenceClass
        $CandidateCounts = @(
            $script:PayloadValidationDiagnostic.ExpectedFiles,
            $script:PayloadValidationDiagnostic.ActualFiles,
            $script:PayloadValidationDiagnostic.ExpectedDirectories,
            $script:PayloadValidationDiagnostic.ActualDirectories
        )
        $CountsAreSafe = $true
        foreach ($CandidateCount in $CandidateCounts) {
            if ($CandidateCount -isnot [int] -or
                $CandidateCount -lt 0 -or
                $CandidateCount -gt $script:WindowsPayloadMaxEntries) {
                $CountsAreSafe = $false
                break
            }
        }
        if ($PayloadDifferenceClasses -ccontains $CandidateDifferenceClass -and
            $CountsAreSafe) {
            $PayloadDifferenceClass = $CandidateDifferenceClass
            $ExpectedFileCount = [int] $CandidateCounts[0]
            $ActualFileCount = [int] $CandidateCounts[1]
            $ExpectedDirectoryCount = [int] $CandidateCounts[2]
            $ActualDirectoryCount = [int] $CandidateCounts[3]
        } else {
            $FailureClass = "powershell_runtime"
            $ExitClass = "failure"
        }
    }

    return [PSCustomObject]@{
        FailureClass = $FailureClass
        Stage = $Stage
        ErrorKind = $ErrorKind
        ElapsedBucket = $ElapsedBucket
        ExitClass = $ExitClass
        AvailableMemoryBucket = $MemoryBucket
        CleanupStatus = $CurrentCleanupStatus
        PayloadDifferenceClass = $PayloadDifferenceClass
        ExpectedFileCount = $ExpectedFileCount
        ActualFileCount = $ActualFileCount
        ExpectedDirectoryCount = $ExpectedDirectoryCount
        ActualDirectoryCount = $ActualDirectoryCount
    }
}

function Write-SanitizedInstallerSmokeFailure($Record) {
    $Line = (
        "WINDOWS_VALIDATED_INSTALLER_SMOKE_FAIL " +
        "failure_class=$($Record.FailureClass) " +
        "stage=$($Record.Stage) " +
        "exit_class=$($Record.ExitClass) " +
        "available_memory_bucket=$($Record.AvailableMemoryBucket) " +
        "cleanup_status=$($Record.CleanupStatus)"
    )
    if ($null -ne $Record.ErrorKind) {
        $Line += " error_kind=$($Record.ErrorKind)"
    }
    if ($null -ne $Record.ElapsedBucket) {
        $Line += " elapsed_bucket=$($Record.ElapsedBucket)"
    }
    if ($null -ne $Record.PayloadDifferenceClass) {
        $Line += (
            " payload_difference=$($Record.PayloadDifferenceClass)" +
            " expected_files=$($Record.ExpectedFileCount)" +
            " actual_files=$($Record.ActualFileCount)" +
            " expected_directories=$($Record.ExpectedDirectoryCount)" +
            " actual_directories=$($Record.ActualDirectoryCount)"
        )
    }
    [Console]::Out.WriteLine($Line)
}

try {
    . (Join-Path $PSScriptRoot "windows-runtime.ps1")
    . (Join-Path $PSScriptRoot "windows-payload.ps1")

    if (-not $AuthorizeDestructiveInstallerSmoke) {
        throw "the validated installer smoke test requires explicit destructive authorization"
    }
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT -or
        -not [Environment]::Is64BitProcess) {
        throw "the validated installer smoke test requires 64-bit Windows"
    }
    $Os = Get-CimInstance Win32_OperatingSystem
    $Processors = @(Get-CimInstance Win32_Processor)
    if ([int] $Os.ProductType -ne 1 -or
        [version] $Os.Version -lt [version] "10.0" -or
        $Processors.Count -eq 0 -or
        @($Processors | Where-Object Architecture -ne 9).Count -ne 0) {
        throw "the validated installer smoke test requires native x64 Windows 10 or 11 client"
    }
    if ([string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
        throw "Windows did not expose the per-user local application data directory"
    }
    $AvailableMemoryBytes = [uint64] $Os.FreePhysicalMemory * 1024
    $AvailableMemoryBucket = if ($AvailableMemoryBytes -lt 2GB) {
        "under_2gib"
    } elseif ($AvailableMemoryBytes -lt 4GB) {
        "2gib_to_4gib"
    } else {
        "over_4gib"
    }

    $InstallerItem = Get-Item -LiteralPath $Installer -ErrorAction Stop
    $BundleItem = Get-Item -LiteralPath $BundleRoot -ErrorAction Stop
    if (-not $InstallerItem.PSIsContainer -and
        $InstallerItem.Extension -ieq ".exe") {
        $Installer = $InstallerItem.FullName
    } else {
        throw "Installer must be one Windows executable"
    }
    if (-not $BundleItem.PSIsContainer) {
        throw "BundleRoot must be a directory"
    }
    $BundleRoot = $BundleItem.FullName
    $InstallDir = Join-Path (Join-Path $env:LOCALAPPDATA "Programs") "AirWiki"
    $UninstallRegistryPath = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\AirWiki"
    $ProductRegistryPath = "HKCU:\Software\AirWiki\AirWiki"
    $AutostartRegistryPath = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
    $AutostartValueName = "AirWiki"
    $DesktopExecutable = Join-Path $InstallDir "airwiki.exe"
    $ExpectedRuntimeExecutable = Join-Path $InstallDir "llama\llama-server.exe"
    $ActivationLogDirectory = Join-Path $env:LOCALAPPDATA "airwiki\AirWiki\data\logs"
    $ActivationStatusPath = Join-Path $ActivationLogDirectory "model-activation-status.json"
    $ActivationStatusCursor = New-ActivationStatusCursor $ActivationStatusPath
    $ActivationLogCursor = New-ActivationLogCursor $ActivationLogDirectory
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

    if ((Test-AnyManagedInstallState) -or
        (@(Get-DesktopProcesses)).Count -ne 0) {
        throw "the validated installer smoke test requires a clean initial state; remove the existing installation manually"
    }

    $script:TerminalStage = "installer"
    $InstalledByThisRun = $true
    Invoke-Process $Installer @("/S", "/NS", "/R") "installer"

    $script:TerminalStage = "registration"
    $RegisteredUninstaller = Wait-ForExactRegisteredUninstaller

    $script:TerminalStage = "desktop_correlation"
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
    $ExpectedDesktopProcessId = [int] $DesktopProcesses[0].ProcessId
    $ExpectedDesktopProcess = [Diagnostics.Process]::GetProcessById(
        $ExpectedDesktopProcessId
    )
    $ExpectedDesktopSafeHandle = $ExpectedDesktopProcess.SafeHandle
    if ($ExpectedDesktopSafeHandle.IsInvalid -or
        $ExpectedDesktopSafeHandle.IsClosed -or
        $ExpectedDesktopProcess.HasExited -or
        -not (Test-SamePath `
            ([string] $ExpectedDesktopProcess.MainModule.FileName) `
            $DesktopExecutable)) {
        throw "the installed desktop process identity is unavailable"
    }

    $script:TerminalStage = "payload_validation"
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
    Set-PayloadValidationFailure $ExpectedInstalled $ActualInstalled
    Assert-WindowsPayloadManifestsEqual `
        $ExpectedInstalled `
        $ActualInstalled `
        "installed application payload"

    $script:TerminalStage = "models"
    Wait-ForModelsReady

    $script:TerminalStage = "uninstall_cleanup"
    Remove-ExactRegisteredInstall
    $InstalledByThisRun = $false
    $CleanupStatus = "pass"

    $script:TerminalStage = "complete"
    $InstallerHash = (
        Get-FileHash -LiteralPath $Installer -Algorithm SHA256
    ).Hash.ToLowerInvariant()
} catch {
    $PrimaryFailed = $true
} finally {
    if ($InstalledByThisRun) {
        $CleanupRequired = $false
        try {
            $CleanupRequired = $script:ProcessTerminationUnconfirmed -or
                (Test-AnyManagedInstallState) -or
                (@(Get-DesktopProcesses)).Count -ne 0
        } catch {
            $PrimaryFailed = $true
            $CleanupStatus = "failed"
        }
        if ($CleanupRequired -and $CleanupStatus -ne "failed") {
            try {
                Invoke-AutomaticCleanup
                $InstalledByThisRun = $false
                $CleanupStatus = "pass"
            } catch {
                $CleanupStatus = "failed"
            }
        }
    }
    if ($null -ne $ExpectedDesktopProcess) {
        $ExpectedDesktopProcess.Dispose()
        $ExpectedDesktopProcess = $null
    }
}

if ($PrimaryFailed -or $CleanupStatus -eq "failed") {
    try {
        $SanitizedFailure = Get-SanitizedInstallerSmokeFailure $CleanupStatus
        Write-SanitizedInstallerSmokeFailure $SanitizedFailure
    } catch {
        [Console]::Out.WriteLine(
            "WINDOWS_VALIDATED_INSTALLER_SMOKE_FAIL failure_class=powershell_runtime stage=unknown exit_class=failure available_memory_bucket=unknown cleanup_status=unknown"
        )
    }
    exit 1
}

[Console]::Out.WriteLine(
    "WINDOWS_VALIDATED_INSTALLER_SMOKE_PASS installer_sha256=$InstallerHash models_ready=pass uninstall=pass"
)
