function Get-AirWikiReleaseVersion(
    [string] $RepositoryRoot,
    [string] $ExpectedVersion = ""
) {
    $Node = (Get-Command node.exe -CommandType Application -ErrorAction Stop).Source
    $Arguments = @((Join-Path $RepositoryRoot "packaging\release-version.mjs"))
    if (-not [string]::IsNullOrWhiteSpace($ExpectedVersion)) {
        $Arguments += @("--expect", $ExpectedVersion)
    }
    $Version = (& $Node @Arguments 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($Version)) {
        throw "AirWiki release version validation failed"
    }
    return $Version
}
