$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$script:WindowsPayloadMaxEntries = 8192
$script:WindowsPayloadMaxBytes = 4GB

function Assert-WindowsPeMachine(
    [string] $Path,
    [UInt16] $ExpectedMachine,
    [string] $Label
) {
    $Verified = Get-VerifiedWindowsRegularFile $Path $Label
    $Stream = [IO.File]::Open($Verified, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        if ($Stream.Length -lt 64) { throw "$Label is not a PE executable" }
        $Reader = [IO.BinaryReader]::new($Stream)
        if ($Reader.ReadUInt16() -ne 0x5a4d) { throw "$Label is not a PE executable" }
        $Stream.Position = 0x3c
        $PeOffset = $Reader.ReadUInt32()
        if ($PeOffset + 6 -gt $Stream.Length) { throw "$Label has a truncated PE header" }
        $Stream.Position = $PeOffset
        if ($Reader.ReadUInt32() -ne 0x00004550) { throw "$Label is not a PE executable" }
        if ($Reader.ReadUInt16() -ne $ExpectedMachine) {
            throw "$Label has an unexpected PE machine"
        }
    } finally {
        $Stream.Dispose()
    }
}

function New-WindowsPayloadManifest {
    return [PSCustomObject]@{
        Files = [Collections.Generic.SortedDictionary[string, object]]::new(
            [StringComparer]::OrdinalIgnoreCase
        )
        Directories = [Collections.Generic.SortedDictionary[string, object]]::new(
            [StringComparer]::OrdinalIgnoreCase
        )
    }
}

function ConvertTo-WindowsPayloadRelativePath([string] $Relative, [string] $Label) {
    if ([string]::IsNullOrWhiteSpace($Relative) -or [IO.Path]::IsPathRooted($Relative)) {
        throw "$Label has an invalid relative path"
    }
    $Segments = @([regex]::Split($Relative, '[\\/]+'))
    foreach ($Segment in $Segments) {
        if ([string]::IsNullOrWhiteSpace($Segment) -or
            $Segment -eq "." -or $Segment -eq "..") {
            throw "$Label has an invalid relative path"
        }
    }
    return [string]::Join("/", $Segments)
}

function Add-WindowsPayloadDirectory(
    [object] $Manifest,
    [string] $Relative,
    [string] $Label
) {
    $Normalized = ConvertTo-WindowsPayloadRelativePath $Relative $Label
    $Segments = $Normalized.Split('/')
    for ($Index = 1; $Index -le $Segments.Length; $Index++) {
        $Directory = [string]::Join("/", $Segments[0..($Index - 1)])
        if (-not $Manifest.Directories.ContainsKey($Directory)) {
            $Manifest.Directories.Add($Directory, $true)
        }
    }
}

function Add-WindowsPayloadFile(
    [object] $Manifest,
    [string] $Relative,
    [string] $Source,
    [string] $Label
) {
    $Normalized = ConvertTo-WindowsPayloadRelativePath $Relative $Label
    $SourcePath = Get-VerifiedWindowsRegularFile $Source $Label
    if ($Manifest.Files.ContainsKey($Normalized)) {
        throw "$Label duplicates payload path $Normalized"
    }
    $Parent = [IO.Path]::GetDirectoryName($Normalized.Replace('/', [IO.Path]::DirectorySeparatorChar))
    if (-not [string]::IsNullOrWhiteSpace($Parent)) {
        Add-WindowsPayloadDirectory $Manifest $Parent $Label
    }
    $Item = Get-Item -LiteralPath $SourcePath -Force -ErrorAction Stop
    $Manifest.Files.Add($Normalized, [PSCustomObject]@{
        Length = [long] $Item.Length
        Sha256 = (Get-FileHash -LiteralPath $SourcePath -Algorithm SHA256).Hash
    })
}

function Add-WindowsPayloadTree(
    [object] $Manifest,
    [string] $Prefix,
    [string] $SourceRoot,
    [string] $Label
) {
    $NormalizedPrefix = ConvertTo-WindowsPayloadRelativePath $Prefix $Label
    Assert-WindowsAbsolutePath $SourceRoot "$Label source root"
    if (-not (Test-Path -LiteralPath $SourceRoot -PathType Container)) {
        throw "$Label source root is missing"
    }
    $RootItem = Get-Item -LiteralPath $SourceRoot -Force -ErrorAction Stop
    Assert-NoWindowsReparseAncestor $RootItem.FullName "$Label source root"
    Add-WindowsPayloadDirectory $Manifest $NormalizedPrefix $Label

    $RootPath = [IO.Path]::GetFullPath($RootItem.FullName).TrimEnd(
        [char[]]@([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
    )
    $RootPrefix = $RootPath + [IO.Path]::DirectorySeparatorChar
    $Pending = [Collections.Generic.Queue[IO.DirectoryInfo]]::new()
    $Pending.Enqueue($RootItem)
    $EntryCount = 0

    while ($Pending.Count -gt 0) {
        $Current = $Pending.Dequeue()
        foreach ($Entry in @(Get-ChildItem -LiteralPath $Current.FullName -Force -ErrorAction Stop)) {
            $EntryCount += 1
            if ($EntryCount -gt $script:WindowsPayloadMaxEntries) {
                throw "$Label source tree contains too many entries"
            }
            if (($Entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "$Label source tree contains a reparse point"
            }
            $FullPath = [IO.Path]::GetFullPath($Entry.FullName)
            if (-not $FullPath.StartsWith($RootPrefix, [StringComparison]::OrdinalIgnoreCase)) {
                throw "$Label source tree escaped its root"
            }
            $SourceRelative = ConvertTo-WindowsPayloadRelativePath `
                $FullPath.Substring($RootPrefix.Length) `
                $Label
            $PayloadRelative = "$NormalizedPrefix/$SourceRelative"
            if ($Entry.PSIsContainer) {
                Add-WindowsPayloadDirectory $Manifest $PayloadRelative $Label
                $Pending.Enqueue($Entry)
            } else {
                Add-WindowsPayloadFile $Manifest $PayloadRelative $Entry.FullName $Label
            }
        }
    }
}

function Get-ActualWindowsPayloadManifest([string] $Root, [string] $Label) {
    Assert-WindowsAbsolutePath $Root "$Label root"
    if (-not (Test-Path -LiteralPath $Root -PathType Container)) {
        throw "$Label root is missing"
    }
    $RootItem = Get-Item -LiteralPath $Root -Force -ErrorAction Stop
    Assert-NoWindowsReparseAncestor $RootItem.FullName "$Label root"
    $Manifest = New-WindowsPayloadManifest
    $RootPath = [IO.Path]::GetFullPath($RootItem.FullName).TrimEnd(
        [char[]]@([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
    )
    $RootPrefix = $RootPath + [IO.Path]::DirectorySeparatorChar
    $Pending = [Collections.Generic.Queue[IO.DirectoryInfo]]::new()
    $Pending.Enqueue($RootItem)
    $EntryCount = 0
    [long] $TotalBytes = 0

    while ($Pending.Count -gt 0) {
        $Current = $Pending.Dequeue()
        foreach ($Entry in @(Get-ChildItem -LiteralPath $Current.FullName -Force -ErrorAction Stop)) {
            $EntryCount += 1
            if ($EntryCount -gt $script:WindowsPayloadMaxEntries) {
                throw "$Label contains too many entries"
            }
            if (($Entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "$Label contains a reparse point"
            }
            $FullPath = [IO.Path]::GetFullPath($Entry.FullName)
            if (-not $FullPath.StartsWith($RootPrefix, [StringComparison]::OrdinalIgnoreCase)) {
                throw "$Label entry escaped its root"
            }
            $Relative = ConvertTo-WindowsPayloadRelativePath `
                $FullPath.Substring($RootPrefix.Length) `
                $Label
            if ($Entry.PSIsContainer) {
                Add-WindowsPayloadDirectory $Manifest $Relative $Label
                $Pending.Enqueue($Entry)
            } else {
                [long] $Length = $Entry.Length
                $TotalBytes += $Length
                if ($TotalBytes -gt $script:WindowsPayloadMaxBytes) {
                    throw "$Label exceeds the size limit"
                }
                if ($Manifest.Files.ContainsKey($Relative)) {
                    throw "$Label contains a duplicate file path"
                }
                $Manifest.Files.Add($Relative, [PSCustomObject]@{
                    Length = $Length
                    Sha256 = (Get-FileHash -LiteralPath $Entry.FullName -Algorithm SHA256).Hash
                })
            }
        }
    }
    return $Manifest
}

function Assert-WindowsPayloadManifestsEqual(
    [object] $Expected,
    [object] $Actual,
    [string] $Label
) {
    if ($Expected.Files.Count -ne $Actual.Files.Count -or
        $Expected.Directories.Count -ne $Actual.Directories.Count) {
        throw "$Label file or directory set differs from the exact release payload"
    }
    foreach ($Relative in $Expected.Directories.Keys) {
        if (-not $Actual.Directories.ContainsKey($Relative)) {
            throw "$Label is missing an expected directory"
        }
    }
    foreach ($Relative in $Expected.Files.Keys) {
        if (-not $Actual.Files.ContainsKey($Relative)) {
            throw "$Label is missing an expected file"
        }
        if ($Expected.Files[$Relative].Length -ne $Actual.Files[$Relative].Length -or
            $Expected.Files[$Relative].Sha256 -ne $Actual.Files[$Relative].Sha256) {
            throw "$Label contains bytes that differ from the exact release payload"
        }
    }
}

function Assert-WindowsPayloadMatches(
    [object] $Expected,
    [string] $ActualRoot,
    [string] $Label
) {
    $Actual = Get-ActualWindowsPayloadManifest $ActualRoot $Label
    Assert-WindowsPayloadManifestsEqual $Expected $Actual $Label
}

function Assert-WindowsDirectoryTreeMatches(
    [string] $ExpectedRoot,
    [string] $ActualRoot,
    [string] $Label
) {
    $Expected = Get-ActualWindowsPayloadManifest $ExpectedRoot "$Label source"
    $Actual = Get-ActualWindowsPayloadManifest $ActualRoot $Label
    Assert-WindowsPayloadManifestsEqual $Expected $Actual $Label
}
