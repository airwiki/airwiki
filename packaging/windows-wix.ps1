Set-StrictMode -Version Latest

function Write-WixLightDiagnostic([string] $Root, [string] $ReleaseDir) {
    $BuildDir = Join-Path $ReleaseDir "wix\x64"
    $Candle = Join-Path $Root "target\.tauri\WixTools314\candle.exe"
    $Light = Join-Path $Root "target\.tauri\WixTools314\light.exe"
    $UiExtension = Join-Path $Root "target\.tauri\WixTools314\WixUIExtension.dll"
    $UtilExtension = Join-Path $Root "target\.tauri\WixTools314\WixUtilExtension.dll"
    $Locale = Join-Path $BuildDir "locale.wxl"
    $Objects = @(Get-ChildItem -LiteralPath $BuildDir -File -Filter *.wixobj -ErrorAction SilentlyContinue)
    $ResourceSource = Join-Path $Root "target\windows-msi-resources.wxs"
    $Desktop = Join-Path $ReleaseDir "airwiki.exe"
    if ((Test-Path -LiteralPath $Candle -PathType Leaf) -and
        (Test-Path -LiteralPath $ResourceSource -PathType Leaf) -and
        (Test-Path -LiteralPath $Desktop -PathType Leaf)) {
        $DiagnosticObject = Join-Path $BuildDir "diagnostic-resources.wixobj"
        Remove-Item -LiteralPath $DiagnosticObject -Force -ErrorAction SilentlyContinue
        try {
            Write-Warning "Tauri did not expose the WiX compiler error; rerunning the pinned compiler for diagnostics"
            & $Candle `
                -arch x64 `
                -out $DiagnosticObject `
                $ResourceSource `
                "-dSourceDir=$Desktop" 2>&1 | ForEach-Object { Write-Warning ([string] $_) }
            $CandleExitCode = $LASTEXITCODE
            Write-Warning "Independent WiX compiler exit code: $CandleExitCode"
        } catch {
            Write-Warning "Could not produce the independent WiX compiler diagnostic: $($_.Exception.Message)"
        } finally {
            Remove-Item -LiteralPath $DiagnosticObject -Force -ErrorAction SilentlyContinue
        }
    }
    if (-not (Test-Path -LiteralPath $Light -PathType Leaf) -or
        -not (Test-Path -LiteralPath $UiExtension -PathType Leaf) -or
        -not (Test-Path -LiteralPath $UtilExtension -PathType Leaf) -or
        -not (Test-Path -LiteralPath $Locale -PathType Leaf) -or
        $Objects.Count -eq 0) {
        Write-Warning "WiX linker inputs were not available for an independent diagnostic"
        return
    }

    $DiagnosticMsi = Join-Path $BuildDir "diagnostic-output.msi"
    $ObjectPaths = @($Objects | ForEach-Object { $_.FullName })
    Remove-Item -LiteralPath $DiagnosticMsi -Force -ErrorAction SilentlyContinue
    try {
        [xml] $LocaleXml = Get-Content -LiteralPath $Locale -Raw
        $Culture = [string] $LocaleXml.WixLocalization.Culture
        if ([string]::IsNullOrWhiteSpace($Culture)) {
            $Culture = "en-us"
        }
        $Cultures = if ($Culture -ieq "en-us") {
            "en-us"
        } else {
            "$($Culture.ToLowerInvariant());en-US"
        }
        Write-Warning "Tauri did not expose the WiX linker error; rerunning the pinned linker for diagnostics"
        Push-Location $BuildDir
        try {
            & $Light `
                -ext $UiExtension `
                -ext $UtilExtension `
                -o $DiagnosticMsi `
                "-cultures:$Cultures" `
                -loc $Locale `
                $ObjectPaths 2>&1 | ForEach-Object { Write-Warning ([string] $_) }
            $LightExitCode = $LASTEXITCODE
        } finally {
            Pop-Location
        }
        Write-Warning "Independent WiX linker exit code: $LightExitCode"
    } catch {
        Write-Warning "Could not produce the independent WiX linker diagnostic: $($_.Exception.Message)"
    } finally {
        Remove-Item -LiteralPath $DiagnosticMsi -Force -ErrorAction SilentlyContinue
    }
}
