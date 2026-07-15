function Test-AirWikiWindowsTraversalSegment([string] $Path) {
    foreach ($Segment in [regex]::Split($Path, '[\\/]+')) {
        if ($Segment -eq "." -or $Segment -eq "..") {
            return $true
        }
    }
    return $false
}

function Get-AirWikiWindowsPathItem([string] $Path, [string] $Label) {
    try {
        return Get-Item -LiteralPath $Path -Force -ErrorAction Stop
    } catch [System.Management.Automation.ItemNotFoundException] {
        return $null
    } catch [IO.FileNotFoundException] {
        return $null
    } catch [IO.DirectoryNotFoundException] {
        return $null
    } catch {
        throw "$Label could not be inspected safely"
    }
}

function Assert-AirWikiWindowsPathHasNoReparsePoint(
    [string] $Path,
    [string] $Label
) {
    $Current = [IO.Path]::GetFullPath($Path)
    while ($null -ne $Current) {
        $Item = Get-AirWikiWindowsPathItem $Current $Label
        if ($null -ne $Item -and
            ($Item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "$Label contains or traverses a reparse point"
        }
        $Parent = [IO.Directory]::GetParent($Current)
        $Current = if ($null -eq $Parent) { $null } else { $Parent.FullName }
    }
}

function Assert-AirWikiWindowsTreeHasNoReparsePoint(
    [string] $Path,
    [string] $Label
) {
    $RootItem = Get-AirWikiWindowsPathItem $Path $Label
    if ($null -eq $RootItem) { return }
    if (($RootItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "$Label contains or traverses a reparse point"
    }
    if (-not $RootItem.PSIsContainer) {
        return
    }

    $Pending = [Collections.Generic.Queue[IO.DirectoryInfo]]::new()
    $Pending.Enqueue($RootItem)
    while ($Pending.Count -gt 0) {
        $Current = $Pending.Dequeue()
        foreach ($Entry in @(Get-ChildItem -LiteralPath $Current.FullName -Force -ErrorAction Stop)) {
            if (($Entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "$Label contains or traverses a reparse point"
            }
            if ($Entry.PSIsContainer) {
                $Pending.Enqueue($Entry)
            }
        }
    }
}

function Remove-AirWikiWindowsStagingPath {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [ValidateNotNullOrEmpty()]
        [string] $Path,

        [Parameter(Mandatory = $true)]
        [ValidateNotNullOrEmpty()]
        [string] $AllowedRoot,

        [Parameter(Mandatory = $true)]
        [ValidateNotNullOrEmpty()]
        [string] $Label
    )

    if ((Test-AirWikiWindowsTraversalSegment $Path) -or
        (Test-AirWikiWindowsTraversalSegment $AllowedRoot)) {
        throw "$Label contains a traversal segment"
    }
    if (-not [IO.Path]::IsPathRooted($Path) -or
        -not [IO.Path]::IsPathRooted($AllowedRoot)) {
        throw "$Label and its allowed root must be absolute paths"
    }

    $Separators = [char[]]@(
        [IO.Path]::DirectorySeparatorChar,
        [IO.Path]::AltDirectorySeparatorChar
    )
    $ResolvedRoot = [IO.Path]::GetFullPath($AllowedRoot)
    $VolumeRoot = [IO.Path]::GetPathRoot($ResolvedRoot)
    if ($ResolvedRoot.Length -gt $VolumeRoot.Length) {
        $ResolvedRoot = $ResolvedRoot.TrimEnd($Separators)
    }
    $AllowedRootItem = Get-AirWikiWindowsPathItem $ResolvedRoot $Label
    if ($null -eq $AllowedRootItem -or -not $AllowedRootItem.PSIsContainer) {
        throw "$Label allowed root is missing or is not a directory"
    }

    $ResolvedPath = [IO.Path]::GetFullPath($Path).TrimEnd($Separators)
    if ($ResolvedPath.Equals($ResolvedRoot, [StringComparison]::OrdinalIgnoreCase)) {
        throw "$Label must be a child of its allowed root"
    }
    $RootPrefix = if ($ResolvedRoot.EndsWith([string] [IO.Path]::DirectorySeparatorChar)) {
        $ResolvedRoot
    } else {
        $ResolvedRoot + [IO.Path]::DirectorySeparatorChar
    }
    if (-not $ResolvedPath.StartsWith($RootPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "$Label escaped its allowed root"
    }

    Assert-AirWikiWindowsPathHasNoReparsePoint $ResolvedRoot $Label
    Assert-AirWikiWindowsPathHasNoReparsePoint $ResolvedPath $Label
    Assert-AirWikiWindowsTreeHasNoReparsePoint $ResolvedPath $Label
    $Item = Get-AirWikiWindowsPathItem $ResolvedPath $Label
    if ($null -eq $Item) {
        return
    }

    # Recheck the path boundary immediately before the destructive operation.
    Assert-AirWikiWindowsPathHasNoReparsePoint $ResolvedPath $Label
    Assert-AirWikiWindowsTreeHasNoReparsePoint $ResolvedPath $Label
    $Item = Get-AirWikiWindowsPathItem $ResolvedPath $Label
    if ($null -eq $Item) { return }
    if (($Item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "$Label contains or traverses a reparse point"
    }
    if ($Item.PSIsContainer) {
        Remove-Item -LiteralPath $ResolvedPath -Recurse -Force -ErrorAction Stop
    } else {
        Remove-Item -LiteralPath $ResolvedPath -Force -ErrorAction Stop
    }
    if ($null -ne (Get-AirWikiWindowsPathItem $ResolvedPath $Label)) {
        throw "$Label could not be removed"
    }
}
