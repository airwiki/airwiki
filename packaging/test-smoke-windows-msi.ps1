$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$Smoke = Get-Content -LiteralPath (Join-Path $Root "packaging\smoke-windows-msi.ps1") -Raw

foreach ($Required in @(
    '[switch] $AuthorizeDestructiveMsiSmoke',
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
foreach ($Forbidden in @('Win32_Product', 'Start-Process -FilePath (Join-Path $InstallDirectory "airwiki.exe")')) {
    if ($Smoke.Contains($Forbidden)) {
        throw "MSI smoke contract contains forbidden behavior: $Forbidden"
    }
}

Write-Host "Windows MSI smoke static contract passed."
