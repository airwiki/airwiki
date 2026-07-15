$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

. (Join-Path $PSScriptRoot "windows-safe-staging.ps1")

function Assert-Rejected([scriptblock] $Action, [string] $Label) {
    try {
        & $Action
    } catch {
        return
    }
    throw "$Label was not rejected"
}

$TempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$TestRoot = Join-Path $TempRoot "airwiki-staging-test-$([Guid]::NewGuid().ToString('N'))"
$Outside = Join-Path $TempRoot "airwiki-staging-outside-$([Guid]::NewGuid().ToString('N'))"
$Junction = Join-Path $TestRoot "junction"
$NestedStage = Join-Path $TestRoot "nested"
$NestedJunction = Join-Path $NestedStage "junction"
$BrokenJunction = Join-Path $TestRoot "broken-junction"

try {
    New-Item -ItemType Directory -Path $TestRoot, $Outside | Out-Null
    [IO.File]::WriteAllText((Join-Path $Outside "sentinel.txt"), "outside")

    Assert-Rejected {
        Remove-AirWikiWindowsStagingPath `
            -Path $TestRoot `
            -AllowedRoot $TestRoot `
            -Label "root equality test"
    } "allowed root equality"
    Assert-Rejected {
        Remove-AirWikiWindowsStagingPath `
            -Path (Join-Path $TestRoot "child\..\other") `
            -AllowedRoot $TestRoot `
            -Label "traversal test"
    } "traversal"
    Assert-Rejected {
        Remove-AirWikiWindowsStagingPath `
            -Path $Outside `
            -AllowedRoot $TestRoot `
            -Label "escape test"
    } "root escape"

    $Safe = Join-Path $TestRoot "safe"
    New-Item -ItemType Directory -Path $Safe | Out-Null
    [IO.File]::WriteAllText((Join-Path $Safe "payload.txt"), "safe")
    Remove-AirWikiWindowsStagingPath `
        -Path $Safe `
        -AllowedRoot $TestRoot `
        -Label "safe staging test"
    if (Test-Path -LiteralPath $Safe) {
        throw "safe staging directory was not removed"
    }
    $SafeFile = Join-Path $TestRoot "safe.tmp"
    [IO.File]::WriteAllText($SafeFile, "safe")
    Remove-AirWikiWindowsStagingPath `
        -Path $SafeFile `
        -AllowedRoot $TestRoot `
        -Label "safe staging file test"
    if (Test-Path -LiteralPath $SafeFile) {
        throw "safe staging file was not removed"
    }
    Remove-AirWikiWindowsStagingPath `
        -Path (Join-Path $TestRoot "already-absent") `
        -AllowedRoot $TestRoot `
        -Label "absent staging test"

    & cmd.exe /d /c "mklink /J `"$Junction`" `"$Outside`"" | Out-Null
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $Junction)) {
        throw "test junction could not be created"
    }
    Assert-Rejected {
        Remove-AirWikiWindowsStagingPath `
            -Path $Junction `
            -AllowedRoot $TestRoot `
            -Label "reparse test"
    } "reparse point"
    if (-not (Test-Path -LiteralPath (Join-Path $Outside "sentinel.txt") -PathType Leaf)) {
        throw "reparse test modified the outside directory"
    }
    & cmd.exe /d /c "rmdir `"$Junction`"" | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "test junction could not be removed"
    }

    New-Item -ItemType Directory -Path $NestedStage | Out-Null
    & cmd.exe /d /c "mklink /J `"$NestedJunction`" `"$Outside`"" | Out-Null
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $NestedJunction)) {
        throw "nested test junction could not be created"
    }
    Assert-Rejected {
        Remove-AirWikiWindowsStagingPath `
            -Path $NestedStage `
            -AllowedRoot $TestRoot `
            -Label "nested reparse test"
    } "nested reparse point"
    if (-not (Test-Path -LiteralPath (Join-Path $Outside "sentinel.txt") -PathType Leaf)) {
        throw "nested reparse test modified the outside directory"
    }
    & cmd.exe /d /c "rmdir `"$NestedJunction`"" | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "nested test junction could not be removed"
    }

    $MissingTarget = Join-Path $Outside "missing-target"
    & cmd.exe /d /c "mklink /J `"$BrokenJunction`" `"$MissingTarget`"" | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "broken test junction could not be created"
    }
    Assert-Rejected {
        Remove-AirWikiWindowsStagingPath `
            -Path $BrokenJunction `
            -AllowedRoot $TestRoot `
            -Label "broken reparse test"
    } "broken reparse point"
    & cmd.exe /d /c "rmdir `"$BrokenJunction`"" | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "broken test junction could not be removed"
    }
} finally {
    if (Test-Path -LiteralPath $Junction) {
        & cmd.exe /d /c "rmdir `"$Junction`"" | Out-Null
    }
    if (Test-Path -LiteralPath $NestedJunction) {
        & cmd.exe /d /c "rmdir `"$NestedJunction`"" | Out-Null
    }
    if ($null -ne (Get-AirWikiWindowsPathItem $BrokenJunction "broken test junction")) {
        & cmd.exe /d /c "rmdir `"$BrokenJunction`"" | Out-Null
    }
    Remove-AirWikiWindowsStagingPath `
        -Path $TestRoot `
        -AllowedRoot $TempRoot `
        -Label "staging test root"
    Remove-AirWikiWindowsStagingPath `
        -Path $Outside `
        -AllowedRoot $TempRoot `
        -Label "staging test outside root"
}

Write-Host "Windows staging cleanup policy tests passed."
