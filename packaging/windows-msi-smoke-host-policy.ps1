$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Test-GitHubHostedWindowsServer2022Context(
    [hashtable] $Environment
) {
    $Expected = [ordered]@{
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
    }
    foreach ($Name in $Expected.Keys) {
        if (-not $Environment.ContainsKey($Name) -or
            [string]$Environment[$Name] -cne [string]$Expected[$Name]) {
            return $false
        }
    }

    if (-not $Environment.ContainsKey("GITHUB_SHA") -or
        -not $Environment.ContainsKey("AIRWIKI_RELEASE_COMMIT")) {
        return $false
    }
    $GitHubSha = [string]$Environment.GITHUB_SHA
    $ReleaseCommit = [string]$Environment.AIRWIKI_RELEASE_COMMIT
    if ($GitHubSha -cnotmatch '^[0-9a-f]{40}$' -or
        $ReleaseCommit -cne $GitHubSha) {
        return $false
    }
    return $true
}

function Assert-WindowsMsiSmokeHostPolicy {
    [CmdletBinding()]
    param(
        [bool] $HasDestructiveAuthorization,
        [bool] $AllowGitHubHostedWindowsServer2022,
        [bool] $IsWindows,
        [bool] $Is64BitProcess,
        [int] $ProductType,
        [version] $OperatingSystemVersion,
        [string] $BuildNumber,
        [int[]] $ProcessorArchitectures,
        [hashtable] $Environment
    )

    if (-not $HasDestructiveAuthorization) {
        throw "the MSI smoke test requires explicit destructive authorization"
    }
    if (-not $IsWindows -or -not $Is64BitProcess) {
        throw "the MSI smoke test requires 64-bit Windows"
    }
    if ($OperatingSystemVersion -lt [version]"10.0" -or
        $ProcessorArchitectures.Count -eq 0 -or
        @($ProcessorArchitectures | Where-Object { $_ -ne 9 }).Count -ne 0) {
        throw "the MSI smoke test requires native x64 Windows version 10.0 or newer"
    }
    if ($ProductType -eq 1) {
        if ($AllowGitHubHostedWindowsServer2022) {
            throw "the workflow-only Windows Server exception must not be used on a Windows client"
        }
        return
    }
    if ($ProductType -ne 3 -or
        $BuildNumber -cne "20348" -or
        -not $AllowGitHubHostedWindowsServer2022 -or
        -not (Test-GitHubHostedWindowsServer2022Context $Environment)) {
        throw "the MSI smoke test requires Windows 10 or 11 client, except for the exact authorized GitHub-hosted Windows Server 2022 release job"
    }
}
