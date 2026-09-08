$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

# Exercise the registration decision without installing anything or touching
# the registry. Native CI additionally checks the COM ProductState accessor.
$Tokens = $null
$ParseErrors = $null
$Ast = [System.Management.Automation.Language.Parser]::ParseFile(
    (Join-Path $PSScriptRoot "smoke-windows-msi.ps1"), [ref]$Tokens, [ref]$ParseErrors
)
if ($ParseErrors.Count -ne 0) { throw "MSI smoke script has parser errors" }
foreach ($Name in @("Get-MsiProductState", "Wait-ForMsiProduct", "Assert-MsiProductRegistration")) {
    $Definitions = @($Ast.FindAll({
        param($Node)
        $Node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $Node.Name -ceq $Name
    }, $false))
    if ($Definitions.Count -ne 1) { throw "Missing unique MSI registration function: $Name" }
    . ([scriptblock]::Create($Definitions[0].Extent.Text))
}

if ([Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT) {
    $UnknownCode = "{$([guid]::NewGuid().ToString().ToUpperInvariant())}"
    if ((Get-MsiProductState $UnknownCode) -ne -1) {
        throw "Windows Installer did not report an unknown synthetic ProductCode"
    }
}

$StateWaitMilliseconds = 0
$ProductName = "AirWiki"
$Publisher = "AirWiki"
$Metadata = [pscustomobject]@{
    ProductCode = "{00000000-0000-0000-0000-000000000001}"
    ProductVersion = [version]"0.3.0"
}
$ExpectedDirectory = "synthetic-install"
$GoodInfo = @{
    InstalledProductName = "AirWiki"
    Publisher = "AirWiki"
    VersionString = "0.3.0"
    AssignmentType = "0"
    InstallLocation = $ExpectedDirectory
}
$Info = $GoodInfo.Clone()
$State = 5
$ProbeFails = $false
$InfoFails = $false
$Observed = @{ InfoReads = 0 }

function Get-MsiProductState([string] $ProductCode) {
    if ($ProductCode -cne $Metadata.ProductCode) { throw "unexpected state probe" }
    if ($ProbeFails) { throw "synthetic state probe failure" }
    return $State
}
function Get-MsiProductInfo([string] $ProductCode) {
    if ($ProductCode -cne $Metadata.ProductCode) { throw "unexpected metadata probe" }
    $Observed.InfoReads++
    if ($InfoFails) { throw "synthetic metadata probe failure" }
    return $Info
}
function Test-SamePath([string] $Left, [string] $Right) { return $Left -ceq $Right }
function Get-MsiOperationState([string] $ProductCode) { return "registrations=0 payload=False shortcut=False msiexec_processes=0" }
function Start-Sleep([int] $Milliseconds) { }
function Test-Path([string] $LiteralPath) { throw "MSI registration must not depend on an Uninstall registry key" }
function Assert-Rejected([scriptblock] $Action, [string] $Expected) {
    try { & $Action } catch {
        if (-not $_.Exception.Message.StartsWith($Expected, [StringComparison]::Ordinal)) { throw }
        return
    }
    throw "Expected rejection: $Expected"
}

# A registered per-user product succeeds even without an HKCU Uninstall entry.
Assert-MsiProductRegistration $Metadata $ExpectedDirectory
if ($Observed.InfoReads -ne 1) { throw "installed registration metadata was not checked" }

foreach ($BadState in @(-7, -6, -5, -4, -3, -2, -1, 0, 1, 2, 3, 4)) {
    $State = $BadState
    Assert-Rejected { Assert-MsiProductRegistration $Metadata $ExpectedDirectory } "MSI registration did not reach the expected state"
}
if ($Observed.InfoReads -ne 1) { throw "metadata was read for a product not known to be installed" }
$State = -1
Wait-ForMsiProduct $Metadata.ProductCode $false
foreach ($ResidualState in @(0, 1, 2, 3, 4, 5)) {
    $State = $ResidualState
    Assert-Rejected { Wait-ForMsiProduct $Metadata.ProductCode $false } "MSI registration did not reach the expected state"
}

$State = 5
foreach ($Case in @(
    @{ Name = "AssignmentType"; Value = "1" },
    @{ Name = "AssignmentType"; Value = "" },
    @{ Name = "InstalledProductName"; Value = "Other product" },
    @{ Name = "Publisher"; Value = "Other publisher" },
    @{ Name = "VersionString"; Value = "0.2.0" },
    @{ Name = "InstallLocation"; Value = "outside-install" },
    @{ Name = "InstallLocation"; Value = "" }
)) {
    $Info = $GoodInfo.Clone()
    $Info[$Case.Name] = $Case.Value
    Assert-Rejected { Assert-MsiProductRegistration $Metadata $ExpectedDirectory } "MSI registration does not identify the installed per-user AirWiki payload"
}
$Info = $GoodInfo.Clone()
$ProbeFails = $true
Assert-Rejected { Assert-MsiProductRegistration $Metadata $ExpectedDirectory } "synthetic state probe failure"
Assert-Rejected { Wait-ForMsiProduct $Metadata.ProductCode $false } "synthetic state probe failure"
$ProbeFails = $false
$InfoFails = $true
Assert-Rejected { Assert-MsiProductRegistration $Metadata $ExpectedDirectory } "synthetic metadata probe failure"

Write-Host "Windows MSI installed state, per-user identity, absence and probe failure checks passed."
