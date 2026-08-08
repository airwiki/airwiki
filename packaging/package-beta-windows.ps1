param(
    [Parameter(Mandatory = $true)]
    [string]$BootstrapFile,
    [switch]$ValidateOnly
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if (-not $env:AIRWIKI_BETA_CANDIDATE_SHA) {
    throw 'AIRWIKI_BETA_CANDIDATE_SHA is required'
}
$bootstrapItem = Get-Item -LiteralPath $BootstrapFile -Force
if (($bootstrapItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or
    -not ($bootstrapItem -is [IO.FileInfo])) {
    throw 'The beta bootstrap must be a regular non-reparse-point file'
}
if ($bootstrapItem.Length -gt 8192) {
    throw 'The beta bootstrap exceeds the bounded registry size'
}
$bootstrap = [IO.File]::ReadAllText($bootstrapItem.FullName)
$bootstrap = $bootstrap -replace '\r?\n$', ''
if ($bootstrap.Contains("`r") -or $bootstrap.Contains("`n")) {
    throw 'The beta bootstrap must contain exactly one line'
}
$entries = $bootstrap.Split(';')
if ($entries.Count -lt 1 -or $entries.Count -gt 2) {
    throw 'The beta bootstrap must contain one or two nodes'
}
$east = $entries[0].Split('|')
if ($east.Count -ne 4 -or $east[0] -notmatch '^[1-9][0-9]*$') {
    throw 'The private bootstrap is invalid'
}
if ($entries.Count -eq 2) {
    $west = $entries[1].Split('|')
    if ($west.Count -ne 4 -or
        $east[0] -ne $west[0] -or
        $east[1] -ne $west[1] -or
        $east[2] -eq $west[2] -or
        $east[3] -eq $west[3]) {
        throw 'The private bootstrap is not one coherent two-node registry'
    }
}

$repositoryRoot = (& git rev-parse --show-toplevel).Trim()
if ($LASTEXITCODE -ne 0) {
    throw 'Could not resolve the repository root'
}
$currentSha = (& git -C $repositoryRoot rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $currentSha -ne $env:AIRWIKI_BETA_CANDIDATE_SHA) {
    throw 'AIRWIKI_BETA_CANDIDATE_SHA does not match HEAD'
}
$status = (& git -C $repositoryRoot status --porcelain --untracked-files=normal)
if ($LASTEXITCODE -ne 0 -or $status) {
    throw 'Beta packaging requires a clean worktree'
}

$bootstrapSha256 = (Get-FileHash -LiteralPath $bootstrapItem.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
if ($ValidateOnly) {
    Write-Output 'Windows beta bootstrap packaging policy: PASS'
    return
}
$previousBootstrap = $env:AIRWIKI_BOOTSTRAP_FEDERATION_INDEXES
try {
    $env:AIRWIKI_BOOTSTRAP_FEDERATION_INDEXES = $bootstrap
    & (Join-Path $repositoryRoot 'packaging/package-windows.ps1')
    if ($LASTEXITCODE -ne 0) {
        throw 'Windows beta packaging failed'
    }
}
finally {
    if ($null -eq $previousBootstrap) {
        Remove-Item Env:AIRWIKI_BOOTSTRAP_FEDERATION_INDEXES -ErrorAction SilentlyContinue
    }
    else {
        $env:AIRWIKI_BOOTSTRAP_FEDERATION_INDEXES = $previousBootstrap
    }
}
Write-Output "Windows x64 beta candidate built from $currentSha"
Write-Output "bootstrap SHA-256: $bootstrapSha256"
