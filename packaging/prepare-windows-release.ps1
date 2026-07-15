$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$LlamaRuntime = Join-Path $Root "resources\llama\windows-x64"
$LlamaPolicy = Join-Path $Root "packaging\llama-windows-build-policy.json"
. (Join-Path $PSScriptRoot "windows-runtime.ps1")

if ([string]::IsNullOrWhiteSpace($env:AIRWIKI_UPDATER_PUBLIC_KEY)) {
    throw "compiled updater public key is required"
}
if ($env:AIRWIKI_UPDATE_ENDPOINT -cne `
    "https://github.com/airwiki/airwiki/releases/latest/download/latest.json") {
    throw "compiled updater endpoint must be AirWiki's stable GitHub Releases manifest"
}

Push-Location $Root
try {
    & cargo run --locked -p xtask -- licenses check
    if ($LASTEXITCODE -ne 0) { throw "license validation failed" }

    & cargo run --locked -p xtask -- packaging verify-windows-uninstaller
    if ($LASTEXITCODE -ne 0) { throw "Windows uninstaller policy validation failed" }

    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File packaging\fetch-llama-windows.ps1
    if ($LASTEXITCODE -ne 0) { throw "llama.cpp runtime source build failed" }
    $LlamaManifest = Get-WindowsLlamaRuntimeManifest $LlamaRuntime $LlamaPolicy
    $env:AIRWIKI_WINDOWS_LLAMA_SERVER_SHA256 = `
        [string] @($LlamaManifest.runtime.files)[0].sha256

    & cargo build --locked --release --target x86_64-pc-windows-msvc `
        -p airwiki-desktop `
        -p airwiki-mcp-bridge `
        -p airwiki-windows-firewall-helper
    if ($LASTEXITCODE -ne 0) { throw "release build failed" }
    Assert-WindowsDesktopEmbedsLlamaRuntimeHash `
        (Join-Path $Root "target\x86_64-pc-windows-msvc\release\airwiki.exe") `
        $LlamaRuntime `
        $LlamaPolicy
} finally {
    Pop-Location
}
