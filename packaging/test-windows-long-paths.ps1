$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

. (Join-Path $PSScriptRoot "windows-runtime.ps1")

$TempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$TestRoot = Join-Path $TempRoot "airwiki-long-path-test-$([Guid]::NewGuid().ToString('N'))"
$ExtendedTestRoot = ConvertTo-WindowsExtendedLengthPath $TestRoot "long-path test root"
$Current = $ExtendedTestRoot

try {
    [IO.Directory]::CreateDirectory($ExtendedTestRoot) | Out-Null
    while ($Current.Length -le 280) {
        $Current = Join-Path $Current "segment-0123456789abcdef"
    }
    [IO.Directory]::CreateDirectory($Current) | Out-Null
    $Sentinel = Join-Path $Current "sentinel.txt"
    [IO.File]::WriteAllText($Sentinel, "synthetic")

    $Entries = @(Get-ChildItem -LiteralPath $ExtendedTestRoot -Force -Recurse)
    if ($Entries.Count -lt 2) {
        throw "extended-length enumeration did not return the synthetic tree"
    }
    $Sentinels = @($Entries | Where-Object { $_.Name -eq "sentinel.txt" })
    if ($Sentinels.Count -ne 1 -or
        ($Sentinels[0].Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "extended-length enumeration returned an invalid sentinel"
    }
    $Attributes = [IO.File]::GetAttributes($Sentinel)
    if (($Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "extended-length attribute inspection misclassified the sentinel"
    }
} finally {
    if ([IO.Directory]::Exists($ExtendedTestRoot)) {
        [IO.Directory]::Delete($ExtendedTestRoot, $true)
    }
}

Write-Host "Windows extended-length path tests passed."
