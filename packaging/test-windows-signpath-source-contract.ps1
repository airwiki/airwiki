$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$Workflow = Get-Content -LiteralPath (Join-Path $Root ".github\workflows\windows-signpath.yml") -Raw
$Policy = Get-Content -LiteralPath (Join-Path $PSScriptRoot "windows-signpath.ps1") -Raw
$Package = Get-Content -LiteralPath (Join-Path $PSScriptRoot "package-signpath-windows-msi.ps1") -Raw

foreach ($Required in @(
    "SIGNPATH_FOUNDATION_ENROLLMENT",
    "SignPath Foundation enrollment is not approved",
    "github.repository == 'airwiki/airwiki' && github.ref == 'refs/heads/main'",
    "signpath/github-action-submit-signing-request@b9d91eadd323de506c0c81cf0c7fe7438f3360fd"
)) {
    if (-not $Workflow.Contains($Required)) { throw "SignPath workflow is missing required gate: $Required" }
}
foreach ($Required in @(
    "verify /pa /all /tw",
    "verify /pa /all /tw /ds 0",
    "verify /pa /all /tw /ds 1",
    "Get-VerifiedWindowsSignTool",
    "Assert-ExpectedSignPathSigner"
)) {
    if (-not ($Policy + $Package).Contains($Required)) { throw "SignPath source is missing required verification: $Required" }
}
