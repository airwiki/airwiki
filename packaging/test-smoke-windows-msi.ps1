$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$SmokePath = Join-Path $Root "packaging\smoke-windows-msi.ps1"
$HostPolicyPath = Join-Path $Root "packaging\windows-msi-smoke-host-policy.ps1"
$RecordAccessPath = Join-Path $Root "packaging\windows-installer-record.ps1"
$ReleaseWorkflowPath = Join-Path $Root ".github\workflows\package-platform-rc.yml"
$CiWorkflowPath = Join-Path $Root ".github\workflows\ci.yml"
$Smoke = Get-Content -LiteralPath $SmokePath -Raw
$HostPolicy = Get-Content -LiteralPath $HostPolicyPath -Raw
$RecordAccess = Get-Content -LiteralPath $RecordAccessPath -Raw
$ReleaseWorkflow = Get-Content -LiteralPath $ReleaseWorkflowPath -Raw
$CiWorkflow = Get-Content -LiteralPath $CiWorkflowPath -Raw

function Assert-PowerShellParses([string] $Path, [string] $Label) {
    $Tokens = $null
    $ParseErrors = $null
    [void][System.Management.Automation.Language.Parser]::ParseFile(
        $Path,
        [ref]$Tokens,
        [ref]$ParseErrors
    )
    if ($ParseErrors.Count -ne 0) {
        $Details = ($ParseErrors | ForEach-Object {
            "$($_.Extent.StartLineNumber):$($_.Extent.StartColumnNumber): $($_.Message)"
        }) -join "; "
        throw "$Label has PowerShell parser errors: $Details"
    }
}

function Assert-Accepted([scriptblock] $Action, [string] $Label) {
    try { & $Action } catch { throw "$Label was rejected: $($_.Exception.Message)" }
}

function Assert-Rejected([scriptblock] $Action, [string] $Label) {
    try {
        & $Action
        throw "$Label was unexpectedly accepted"
    } catch {
        if ($_.Exception.Message -eq "$Label was unexpectedly accepted") { throw }
    }
}

function Copy-Hashtable([hashtable] $Source) {
    $Copy = @{}
    foreach ($Key in $Source.Keys) { $Copy[$Key] = $Source[$Key] }
    return $Copy
}

Assert-PowerShellParses $SmokePath "MSI smoke script"
Assert-PowerShellParses $HostPolicyPath "MSI smoke host policy"
Assert-PowerShellParses $RecordAccessPath "Windows Installer record access helper"
& (Join-Path $PSScriptRoot "test-windows-msi-registration.ps1")
& (Join-Path $PSScriptRoot "test-windows-msi-state.ps1")
. $HostPolicyPath
. $RecordAccessPath

function Set-WindowsInstallerRecordStringDataForTest {
    param(
        [Parameter(Mandatory = $true)]
        [object] $Record,

        [Parameter(Mandatory = $true)]
        [int] $Field,

        [Parameter(Mandatory = $true)]
        [string] $Value
    )

    [void] $Record.GetType().InvokeMember(
        "StringData",
        [System.Reflection.BindingFlags]::SetProperty,
        $null,
        $Record,
        [object[]]@($Field, $Value)
    )
}

function Assert-WindowsInstallerRecordAccess {
    $InstallerCom = $null
    $Record = $null
    try {
        $InstallerCom = New-Object -ComObject WindowsInstaller.Installer
        $Record = $InstallerCom.CreateRecord(2)
        Set-WindowsInstallerRecordStringDataForTest -Record $Record -Field 1 -Value "ProductCode"
        Set-WindowsInstallerRecordStringDataForTest `
            -Record $Record `
            -Field 2 `
            -Value "{00000000-0000-0000-0000-000000000001}"

        $Name = Get-WindowsInstallerRecordStringData -Record $Record -Field 1
        $Value = Get-WindowsInstallerRecordStringData -Record $Record -Field 2
        if ($Name -cne "ProductCode" -or
            $Value -cne "{00000000-0000-0000-0000-000000000001}") {
            throw "Windows Installer StringData returned unexpected field values"
        }
    } finally {
        if ($null -ne $Record) {
            [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($Record)
        }
        if ($null -ne $InstallerCom) {
            [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($InstallerCom)
        }
    }
}

Assert-WindowsInstallerRecordAccess

function Assert-WindowsInstallerMetadataRead {
    # Load only the read-only metadata functions, never the install/uninstall entrypoint.
    $Tokens = $null
    $ParseErrors = $null
    $SmokeAst = [System.Management.Automation.Language.Parser]::ParseFile(
        $SmokePath, [ref]$Tokens, [ref]$ParseErrors
    )
    foreach ($FunctionName in @(
        "Assert-RegularFile", "ConvertTo-MsiGuid", "Get-MsiProperties", "Get-InstallerMetadata"
    )) {
        $Definitions = @($SmokeAst.FindAll({
            param($Node)
            $Node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
                $Node.Name -ceq $FunctionName
        }, $false))
        if ($Definitions.Count -ne 1) { throw "Missing unique MSI metadata function: $FunctionName" }
        . ([scriptblock]::Create($Definitions[0].Extent.Text))
    }

    $ProductName = "AirWiki"
    $Publisher = "AirWiki"
    $Expected = @{
        ProductCode = "{00000000-0000-0000-0000-000000000001}"
        UpgradeCode = "{00000000-0000-0000-0000-000000000002}"
        ProductVersion = "0.3.0"
        ProductName = $ProductName
        Manufacturer = $Publisher
    }
    $FixtureRoot = Join-Path ([IO.Path]::GetTempPath()) ("airwiki-msi-metadata-" + [guid]::NewGuid())
    $null = New-Item -ItemType Directory -Path $FixtureRoot

    function New-MetadataFixture([string] $Path, [hashtable] $Properties) {
        $InstallerCom = $null
        $Database = $null
        $View = $null
        $Record = $null
        try {
            $InstallerCom = New-Object -ComObject WindowsInstaller.Installer
            $Database = $InstallerCom.OpenDatabase($Path, 3)
            $View = $Database.OpenView('CREATE TABLE `Property` (`Property` CHAR(72) NOT NULL, `Value` CHAR(0) LOCALIZABLE PRIMARY KEY `Property`)')
            [void] $View.Execute()
            [void] $View.Close()
            [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($View)
            $View = $null
            $View = $Database.OpenView('INSERT INTO `Property` (`Property`, `Value`) VALUES (?, ?)')
            $Record = $InstallerCom.CreateRecord(2)
            foreach ($Name in $Properties.Keys) {
                Set-WindowsInstallerRecordStringDataForTest -Record $Record -Field 1 -Value $Name
                Set-WindowsInstallerRecordStringDataForTest -Record $Record -Field 2 -Value $Properties[$Name]
                [void] $View.Execute($Record)
            }
            [void] $Database.Commit()
        } finally {
            if ($null -ne $Record) { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($Record) }
            if ($null -ne $View) { [void] $View.Close(); [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($View) }
            if ($null -ne $Database) { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($Database) }
            if ($null -ne $InstallerCom) { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($InstallerCom) }
        }
    }

    try {
        $Fixture = Join-Path $FixtureRoot "valid.msi"
        New-MetadataFixture $Fixture $Expected
        $Results = @(Get-MsiProperties $Fixture)
        if ($Results.Count -ne 1 -or $Results[0] -isnot [hashtable]) {
            throw "MSI metadata reader must return exactly one property map"
        }
        foreach ($Name in $Expected.Keys) {
            if ($Results[0][$Name] -cne $Expected[$Name]) { throw "MSI property did not round-trip: $Name" }
        }
        $Metadata = Get-InstallerMetadata $Fixture
        if ($Metadata.ProductCode -cne $Expected.ProductCode -or
            $Metadata.UpgradeCode -cne $Expected.UpgradeCode -or
            $Metadata.ProductVersion -ne [version]$Expected.ProductVersion -or
            $Metadata.Sha256 -cne (Get-FileHash -LiteralPath $Fixture -Algorithm SHA256).Hash.ToLowerInvariant()) {
            throw "MSI metadata reader returned an unexpected identity or digest"
        }
        $Invalid = Copy-Hashtable $Expected
        $Invalid.ProductCode = "not-a-guid"
        $InvalidFixture = Join-Path $FixtureRoot "invalid-guid.msi"
        New-MetadataFixture $InvalidFixture $Invalid
        Assert-Rejected { Get-InstallerMetadata $InvalidFixture } "Invalid MSI product GUID"

        $Missing = Copy-Hashtable $Expected
        $Missing.Remove("Manufacturer")
        $MissingFixture = Join-Path $FixtureRoot "missing-property.msi"
        New-MetadataFixture $MissingFixture $Missing
        Assert-Rejected { Get-InstallerMetadata $MissingFixture } "Missing MSI manufacturer"
    } finally {
        Remove-Item -LiteralPath $FixtureRoot -Recurse -Force
    }
}

Assert-WindowsInstallerMetadataRead

foreach ($Required in @(
    '[switch] $AuthorizeDestructiveMsiSmoke',
    '[switch] $AllowGitHubHostedWindowsServer2022',
    'windows-msi-smoke-host-policy.ps1',
    'windows-installer-record.ps1',
    'Assert-WindowsMsiSmokeHostPolicy',
    'Get-WindowsInstallerRecordStringData',
    'New-Object -ComObject WindowsInstaller.Installer',
    'AUTOLAUNCHAPP=0',
    'Test-WebView2Present',
    'Assert-CleanPreflight',
    '[Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)',
    '[Environment]::GetFolderPath([Environment+SpecialFolder]::ApplicationData)',
    '[Environment]::GetFolderPath([Environment+SpecialFolder]::Programs)',
    'ConvertTo-MsiGuid',
    'Assert-NoProductCodeRegistration',
    'Assert-MsiProductNotInstalled',
    'Assert-PathInsideRoot',
    'ConvertTo-MsiCommandLine',
    '[IO.FileMode]::CreateNew',
    '$script:MarkerDirectories',
    'New-OwnedMarkerDirectories',
    'Remove-EmptyOwnedMarkerDirectories',
    'RemoveAt($Index)',
    '$script:ManualCleanupRequired = $true',
    'Get-MsiOperationState',
    '$null = Assert-InstalledProduct $Metadata',
    'Get-FileHash -LiteralPath $Marker.Path -Algorithm SHA256',
    'Remove-ExactMarkers',
    'ProductVersion -le $Base.ProductVersion'
)) {
    if (-not $Smoke.Contains($Required)) {
        throw "MSI smoke contract is missing: $Required"
    }
}
foreach ($Required in @(
    'ProductType -ne 3',
    'BuildNumber -cne "20348"',
    'GITHUB_ACTIONS = "true"',
    'RUNNER_ENVIRONMENT = "github-hosted"',
    'RUNNER_OS = "Windows"',
    'RUNNER_ARCH = "X64"',
    'GITHUB_SERVER_URL = "https://github.com"',
    'GITHUB_REPOSITORY = "airwiki/airwiki"',
    'GITHUB_EVENT_NAME = "workflow_dispatch"',
    'GITHUB_REF = "refs/heads/main"',
    'GITHUB_REF_TYPE = "branch"',
    'GITHUB_REF_NAME = "main"',
    'GITHUB_JOB = "windows-x64-unsigned-beta"',
    'GITHUB_WORKFLOW_REF = "airwiki/airwiki/.github/workflows/package-platform-rc.yml@refs/heads/main"',
    'AIRWIKI_RELEASE_COMMIT'
)) {
    if (-not $HostPolicy.Contains($Required)) {
        throw "MSI smoke host policy is missing: $Required"
    }
}
foreach ($Forbidden in @('Win32_Product', 'Start-Process -FilePath (Join-Path $InstallDirectory "airwiki.exe")')) {
    if ($Smoke.Contains($Forbidden)) {
        throw "MSI smoke contract contains forbidden behavior: $Forbidden"
    }
}
if ($Smoke.Contains('.StringData(')) {
    throw "MSI smoke contract must use the explicit Windows Installer record accessor"
}
foreach ($Required in @(
    'function Get-WindowsInstallerRecordStringData',
    '[System.Reflection.BindingFlags]::GetProperty',
    '[object[]]@($Field)'
)) {
    if (-not $RecordAccess.Contains($Required)) {
        throw "Windows Installer record access contract is missing: $Required"
    }
}

$WorkflowOptIn = '-AllowGitHubHostedWindowsServer2022'
$TriggerBlock = [regex]::Match(
    $ReleaseWorkflow,
    '(?ms)^on:\r?\n(?<body>.*?)(?=^permissions:\r?\n)'
)
$TriggerNames = @([regex]::Matches(
    $TriggerBlock.Groups['body'].Value,
    '(?m)^  ([A-Za-z][A-Za-z0-9_-]*):\s*\r?$'
) | ForEach-Object { $_.Groups[1].Value })
if (-not $TriggerBlock.Success -or
    $TriggerNames.Count -ne 1 -or
    $TriggerNames[0] -cne 'workflow_dispatch') {
    throw "platform RC workflow must remain manual-dispatch only"
}

$PermissionBlock = [regex]::Match(
    $ReleaseWorkflow,
    '(?ms)^permissions:\r?\n(?<body>.*?)(?=^concurrency:\r?\n)'
)
$RootPermissions = @([regex]::Matches(
    $PermissionBlock.Groups['body'].Value,
    '(?m)^  ([a-z-]+):\s*([a-z]+)\s*\r?$'
) | ForEach-Object { "$($_.Groups[1].Value)=$($_.Groups[2].Value)" })
if (-not $PermissionBlock.Success -or
    $RootPermissions.Count -ne 3 -or
    $RootPermissions -cnotcontains 'actions=read' -or
    $RootPermissions -cnotcontains 'checks=read' -or
    $RootPermissions -cnotcontains 'contents=read') {
    throw "platform RC workflow root permissions must remain the exact read-only set"
}

$WindowsJobMatch = [regex]::Match(
    $ReleaseWorkflow,
    '(?ms)^  windows-x64-unsigned-beta:\r?\n(?<body>.*?)(?=^  publish-platform-rc:\r?\n)'
)
$WindowsJob = $WindowsJobMatch.Groups['body'].Value
if (-not $WindowsJobMatch.Success -or
    [regex]::Matches($ReleaseWorkflow, [regex]::Escape($WorkflowOptIn)).Count -ne 1 -or
    [regex]::Matches($WindowsJob, [regex]::Escape($WorkflowOptIn)).Count -ne 1 -or
    -not $WindowsJob.Contains('runs-on: windows-2022') -or
    -not $WindowsJob.Contains('GITHUB_REF -cne "refs/heads/main"') -or
    -not $WindowsJob.Contains('persist-credentials: false') -or
    -not $WindowsJob.Contains('$Commit -cne $env:AIRWIKI_RELEASE_COMMIT') -or
    -not $WindowsJob.Contains('$Commit -cne $env:GITHUB_SHA') -or
    -not $WindowsJob.Contains('$Commit -cne $Main') -or
    -not $WindowsJob.Contains('git status --porcelain') -or
    $WindowsJob -match '(?m)^    (environment|permissions|secrets):') {
    throw "platform RC workflow does not keep the Windows Server exception on its exact protected job"
}
if ($CiWorkflow.Contains($WorkflowOptIn)) {
    throw "pull-request CI must not opt into destructive Windows Server MSI smoke"
}

$Commit = "a" * 40
$HostedEnvironment = @{
    GITHUB_ACTIONS = "true"
    RUNNER_ENVIRONMENT = "github-hosted"
    RUNNER_OS = "Windows"
    RUNNER_ARCH = "X64"
    GITHUB_SERVER_URL = "https://github.com"
    GITHUB_REPOSITORY = "airwiki/airwiki"
    GITHUB_EVENT_NAME = "workflow_dispatch"
    GITHUB_REF = "refs/heads/main"
    GITHUB_REF_TYPE = "branch"
    GITHUB_REF_NAME = "main"
    GITHUB_JOB = "windows-x64-unsigned-beta"
    GITHUB_WORKFLOW_REF = "airwiki/airwiki/.github/workflows/package-platform-rc.yml@refs/heads/main"
    GITHUB_SHA = $Commit
    AIRWIKI_RELEASE_COMMIT = $Commit
}
$ClientCase = @{
    HasDestructiveAuthorization = $true
    AllowGitHubHostedWindowsServer2022 = $false
    IsWindows = $true
    Is64BitProcess = $true
    ProductType = 1
    OperatingSystemVersion = [version]"10.0.19045"
    BuildNumber = "19045"
    ProcessorArchitectures = [int[]]@(9)
    Environment = @{}
}
$ServerCase = Copy-Hashtable $ClientCase
$ServerCase.AllowGitHubHostedWindowsServer2022 = $true
$ServerCase.ProductType = 3
$ServerCase.OperatingSystemVersion = [version]"10.0.20348"
$ServerCase.BuildNumber = "20348"
$ServerCase.Environment = $HostedEnvironment

Assert-Accepted { Assert-WindowsMsiSmokeHostPolicy @ClientCase } "Windows client host"
$ClientWithServerOptIn = Copy-Hashtable $ClientCase
$ClientWithServerOptIn.AllowGitHubHostedWindowsServer2022 = $true
Assert-Rejected { Assert-WindowsMsiSmokeHostPolicy @ClientWithServerOptIn } "Windows client with workflow-only Server opt-in"
$Windows11Case = Copy-Hashtable $ClientCase
$Windows11Case.OperatingSystemVersion = [version]"10.0.22631"
$Windows11Case.BuildNumber = "22631"
Assert-Accepted { Assert-WindowsMsiSmokeHostPolicy @Windows11Case } "Windows 11 client host"
Assert-Accepted { Assert-WindowsMsiSmokeHostPolicy @ServerCase } "exact GitHub-hosted Server 2022 job"

$NoServerOptIn = Copy-Hashtable $ServerCase
$NoServerOptIn.AllowGitHubHostedWindowsServer2022 = $false
Assert-Rejected { Assert-WindowsMsiSmokeHostPolicy @NoServerOptIn } "Server without explicit opt-in"

foreach ($Build in @("17763", "26100")) {
    $WrongServer = Copy-Hashtable $ServerCase
    $WrongServer.BuildNumber = $Build
    Assert-Rejected { Assert-WindowsMsiSmokeHostPolicy @WrongServer } "unexpected Windows Server build $Build"
}
foreach ($ProductType in @(2, 4)) {
    $WrongProduct = Copy-Hashtable $ServerCase
    $WrongProduct.ProductType = $ProductType
    Assert-Rejected { Assert-WindowsMsiSmokeHostPolicy @WrongProduct } "unexpected Windows ProductType $ProductType"
}

foreach ($Name in @($HostedEnvironment.Keys)) {
    $MissingEnvironment = Copy-Hashtable $HostedEnvironment
    $null = $MissingEnvironment.Remove($Name)
    $MissingCase = Copy-Hashtable $ServerCase
    $MissingCase.Environment = $MissingEnvironment
    Assert-Rejected { Assert-WindowsMsiSmokeHostPolicy @MissingCase } "Server context missing $Name"

    $AlteredEnvironment = Copy-Hashtable $HostedEnvironment
    $AlteredEnvironment[$Name] = "unexpected"
    $AlteredCase = Copy-Hashtable $ServerCase
    $AlteredCase.Environment = $AlteredEnvironment
    Assert-Rejected { Assert-WindowsMsiSmokeHostPolicy @AlteredCase } "Server context altering $Name"
}

$WrongArchitecture = Copy-Hashtable $ServerCase
$WrongArchitecture.ProcessorArchitectures = [int[]]@(12)
Assert-Rejected { Assert-WindowsMsiSmokeHostPolicy @WrongArchitecture } "non-x64 Server processor"
$MixedArchitecture = Copy-Hashtable $ServerCase
$MixedArchitecture.ProcessorArchitectures = [int[]]@(9, 12)
Assert-Rejected { Assert-WindowsMsiSmokeHostPolicy @MixedArchitecture } "mixed Server processor architecture"
$NoProcessors = Copy-Hashtable $ServerCase
$NoProcessors.ProcessorArchitectures = [int[]]@()
Assert-Rejected { Assert-WindowsMsiSmokeHostPolicy @NoProcessors } "Server without a processor"
$NoAuthorization = Copy-Hashtable $ServerCase
$NoAuthorization.HasDestructiveAuthorization = $false
Assert-Rejected { Assert-WindowsMsiSmokeHostPolicy @NoAuthorization } "Server without destructive authorization"
$NotWindows = Copy-Hashtable $ServerCase
$NotWindows.IsWindows = $false
Assert-Rejected { Assert-WindowsMsiSmokeHostPolicy @NotWindows } "non-Windows host"
$Not64Bit = Copy-Hashtable $ServerCase
$Not64Bit.Is64BitProcess = $false
Assert-Rejected { Assert-WindowsMsiSmokeHostPolicy @Not64Bit } "32-bit process"
$OldWindows = Copy-Hashtable $ClientCase
$OldWindows.OperatingSystemVersion = [version]"6.3.9600"
Assert-Rejected { Assert-WindowsMsiSmokeHostPolicy @OldWindows } "pre-Windows-10 client"

Write-Host "Windows MSI smoke COM access, host policy, syntax, and static contract passed."
