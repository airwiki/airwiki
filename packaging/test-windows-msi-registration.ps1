$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

# Execute the production registration guards with synthetic probes. Do not load
# the smoke entrypoint or invoke Windows Installer, the registry or the filesystem.
$Tokens = $null
$ParseErrors = $null
$SmokeAst = [System.Management.Automation.Language.Parser]::ParseFile(
    (Join-Path $PSScriptRoot "smoke-windows-msi.ps1"), [ref]$Tokens, [ref]$ParseErrors
)
if ($ParseErrors.Count -ne 0) { throw "MSI smoke script has parser errors" }
foreach ($Name in @("Assert-NoProductCodeRegistration", "Assert-CleanPreflight", "Remove-InstalledProduct")) {
    $Definitions = @($SmokeAst.FindAll({
        param($Node)
        $Node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $Node.Name -ceq $Name
    }, $false))
    if ($Definitions.Count -ne 1) { throw "Missing unique MSI registration guard: $Name" }
    . ([scriptblock]::Create($Definitions[0].Extent.Text))
}

$Metadata = [pscustomobject]@{ ProductCode = "{00000000-0000-0000-0000-000000000001}" }
$InstallDirectory = "synthetic-install"
$ShortcutPath = "synthetic-shortcut"
$LocalDataRoot = "synthetic-local-root"
$ProgramsRoot = "synthetic-programs-root"
$ArpEntries = @()
$RegistrationPaths = @()
$ArpProbeFails = $false
$RegistrationProbeFails = $false
$InstalledIdentityFails = $false
$Observed = @{ ProductStateChecks = 0; Uninstalls = 0; Stage = "" }

function Get-AirWikiArpEntries {
    if ($ArpProbeFails) { throw "synthetic ARP probe failure" }
    return $ArpEntries
}
function Get-ProductRegistrationPaths([string] $ProductCode) {
    if ($ProductCode -cne $Metadata.ProductCode) { throw "unexpected ProductCode probe" }
    if ($RegistrationProbeFails) { throw "synthetic registration probe failure" }
    return $RegistrationPaths
}
function Assert-PathInsideRoot([string] $Path, [string] $Root, [string] $Label) { return $Path }
function Test-WebView2Present { return $true }
function Test-Path([string] $LiteralPath) { return $false }
function Get-CimInstance([string] $ClassName, [string] $Filter) { return @() }
function Assert-MsiProductNotInstalled($Value) { $Observed.ProductStateChecks++ }
function Assert-InstalledProduct($Value) {
    if ($InstalledIdentityFails) { throw "synthetic installed identity mismatch" }
    $Observed.Stage = "verified"
    return $true
}
function Invoke-MsiExec([string[]] $Arguments, [string] $Label, [string] $ProductCode) {
    if ($Observed.Stage -cne "verified" -or $Label -cne "MSI uninstall" -or
        $ProductCode -cne $Metadata.ProductCode -or $Arguments.Count -ne 4 -or
        $Arguments[0] -cne "/x" -or $Arguments[1] -cne $Metadata.ProductCode -or
        $Arguments[2] -cne "/qn" -or $Arguments[3] -cne "/norestart") {
        throw "unexpected synthetic MSI operation"
    }
    $Observed.Uninstalls++
    $Observed.Stage = "uninstalled"
}
function Wait-ForMsiProduct([string] $ProductCode, [bool] $Present) {
    if ($Observed.Stage -cne "uninstalled" -or $ProductCode -cne $Metadata.ProductCode -or $Present) {
        throw "unexpected synthetic MSI state wait"
    }
    $Observed.Stage = "absent"
}
function Assert-RejectedWithMessage([scriptblock] $Action, [string] $Expected) {
    try { & $Action } catch {
        if ($_.Exception.Message -cne $Expected) { throw }
        return
    }
    throw "Expected rejection: $Expected"
}

Assert-NoProductCodeRegistration $Metadata
Assert-CleanPreflight $Metadata
if ($Observed.ProductStateChecks -ne 1 -or $Observed.Uninstalls -ne 0) {
    throw "empty registration preflight skipped a guard or invoked an installer"
}

foreach ($Count in @(1, 2)) {
    $ArpEntries = @(1..$Count | ForEach-Object { [pscustomobject]@{ Path = "synthetic-arp-$_" } })
    Assert-RejectedWithMessage { Assert-CleanPreflight $Metadata } `
        "the MSI smoke test requires no existing AirWiki installer, payload, or shortcut collision"
    $ArpEntries = @()
    $RegistrationPaths = @(1..$Count | ForEach-Object { "synthetic-registration-$_" })
    Assert-RejectedWithMessage { Assert-NoProductCodeRegistration $Metadata } `
        "the MSI ProductCode already has a registered installation; resolve it manually"
    Assert-RejectedWithMessage { Assert-CleanPreflight $Metadata } `
        "the MSI ProductCode already has a registered installation; resolve it manually"
    $RegistrationPaths = @()
}
$ArpProbeFails = $true
Assert-RejectedWithMessage { Assert-CleanPreflight $Metadata } "synthetic ARP probe failure"
$ArpProbeFails = $false
$RegistrationProbeFails = $true
Assert-RejectedWithMessage { Assert-NoProductCodeRegistration $Metadata } "synthetic registration probe failure"
Assert-RejectedWithMessage { Assert-CleanPreflight $Metadata } "synthetic registration probe failure"
$RegistrationProbeFails = $false
if ($Observed.ProductStateChecks -ne 1 -or $Observed.Uninstalls -ne 0) {
    throw "a preflight collision or probe failure continued past its registration guard"
}

foreach ($Case in @("empty", "one", "multiple", "probe-failure")) {
    $RegistrationPaths = switch ($Case) {
        "one" { "synthetic-registration-1" }
        "multiple" { "synthetic-registration-1"; "synthetic-registration-2" }
        default { @() }
    }
    $RegistrationProbeFails = $Case -ceq "probe-failure"
    $script:InstalledProduct = $Metadata
    $script:ManualCleanupRequired = $false
    $Observed.Stage = ""
    $Observed.Uninstalls = 0
    if ($Case -ceq "empty") {
        Remove-InstalledProduct $Metadata
        if ($null -ne $script:InstalledProduct -or $script:ManualCleanupRequired) {
            throw "successful synthetic uninstall did not clear its owned installation state"
        }
    } else {
        $Expected = if ($RegistrationProbeFails) { "synthetic registration probe failure" } else {
            "MSI uninstall left a ProductCode registration"
        }
        Assert-RejectedWithMessage { Remove-InstalledProduct $Metadata } $Expected
        if ($script:InstalledProduct -ne $Metadata -or -not $script:ManualCleanupRequired) {
            throw "ambiguous synthetic uninstall did not preserve its state for manual cleanup"
        }
    }
    if ($Observed.Uninstalls -ne 1 -or $Observed.Stage -cne "absent") {
        throw "synthetic uninstall did not verify identity, uninstall and wait in order"
    }
}

$InstalledIdentityFails = $true
$script:InstalledProduct = $Metadata
$script:ManualCleanupRequired = $false
$Observed.Uninstalls = 0
Assert-RejectedWithMessage { Remove-InstalledProduct $Metadata } "synthetic installed identity mismatch"
if ($Observed.Uninstalls -ne 0 -or $script:InstalledProduct -ne $Metadata -or -not $script:ManualCleanupRequired) {
    throw "an unverified installation was removed or lost its manual cleanup state"
}

Write-Host "Windows MSI empty, single and multiple registration guards passed."
