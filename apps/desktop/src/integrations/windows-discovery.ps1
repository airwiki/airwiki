$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$pathEntries = @(
    foreach ($scope in @('Machine', 'User')) {
        $value = [Environment]::GetEnvironmentVariable('Path', $scope)
        if ($value) {
            [Environment]::ExpandEnvironmentVariables($value) -split ';' |
                Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
        }
    }
)
$packages = @(
    foreach ($packageName in @('OpenAI.Codex', 'OpenAI.ChatGPT-Desktop', 'Claude')) {
        foreach ($package in @(Get-AppxPackage -Name $packageName -ErrorAction Stop)) {
            $manifest = Get-AppxPackageManifest -Package $package.PackageFullName -ErrorAction Stop
            [PSCustomObject]@{
                name = $package.Name
                location = $package.InstallLocation
                executables = @($manifest.Package.Applications.Application | ForEach-Object { [string]$_.Executable })
            }
        }
    }
)
[PSCustomObject]@{ path_entries = $pathEntries; packages = $packages } |
    ConvertTo-Json -Depth 4 -Compress
