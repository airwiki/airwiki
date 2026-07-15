[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

. (Join-Path $PSScriptRoot "windows-runtime.ps1")

$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$PolicyPath = Join-Path $PSScriptRoot "llama-windows-build-policy.json"
$Policy = Get-Content -LiteralPath $PolicyPath -Raw | ConvertFrom-Json
$Cache = Join-Path $Root "target\packaging-cache"
$Archive = Join-Path $Cache $Policy.source.filename
$RunId = [Guid]::NewGuid().ToString("N").Substring(0, 8)
$WorkParent = Join-Path $Root "target\lw"
$WorkRoot = Join-Path $WorkParent $RunId
$ExtractRoot = Join-Path $WorkRoot "source-archive"
$Source = Join-Path $ExtractRoot $Policy.source.root_directory
$Build = Join-Path $WorkRoot "build"
$RuntimeStage = Join-Path $WorkRoot "runtime"
$Destination = Join-Path $Root "resources\llama\windows-x64"
$ManifestName = "BUILD-MANIFEST.json"
$InternalSingleBuildEnv = "AIRWIKI_LLAMA_INTERNAL_SINGLE_BUILD"

function Get-LowerSha256([string] $Path) {
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Assert-NoReparseAncestor([string] $Path, [string] $Label) {
    $Current = [IO.Path]::GetFullPath($Path)
    while ($null -ne $Current) {
        $Found = $false
        $Attributes = [IO.FileAttributes] 0
        try {
            $Attributes = [IO.File]::GetAttributes($Current)
            $Found = $true
        } catch [IO.FileNotFoundException] {
            # Missing descendants are allowed; their first existing ancestor is
            # still inspected before the path is created.
        } catch [IO.DirectoryNotFoundException] {
            # See the FileNotFoundException branch above.
        } catch {
            throw "$Label or one of its existing ancestors could not be inspected"
        }
        if ($Found -and
            ($Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "$Label or one of its existing ancestors is a reparse point"
        }
        $Parent = [IO.Directory]::GetParent($Current)
        if ($null -eq $Parent) {
            $Current = $null
        } else {
            $Current = $Parent.FullName
        }
    }
}

function Remove-DirectoryWithRetry([string] $Path, [string] $Label) {
    for ($Attempt = 1; $Attempt -le 5; $Attempt += 1) {
        Assert-NoReparseAncestor $Path $Label
        if (-not (Test-Path -LiteralPath $Path)) {
            return
        }
        try {
            Remove-Item -LiteralPath $Path -Recurse -Force -ErrorAction Stop
            return
        } catch {
            if ($Attempt -eq 5) {
                throw "$Label could not be removed after bounded retries"
            }
            Start-Sleep -Milliseconds 500
        }
    }
}

function Assert-RegularFile([string] $Path, [string] $Label) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "$Label is missing or is not a regular file"
    }
    $Item = Get-Item -LiteralPath $Path -Force
    if (($Item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "$Label is a reparse point"
    }
    return $Item
}

function Assert-MicrosoftSignedTool([string] $Path, [string] $Label) {
    $Item = Assert-RegularFile $Path $Label
    $Signature = Get-AuthenticodeSignature -LiteralPath $Item.FullName
    if ($Signature.Status -ne [System.Management.Automation.SignatureStatus]::Valid -or
        $null -eq $Signature.SignerCertificate -or
        $Signature.SignerCertificate.Subject -notmatch '(^|, )O=Microsoft Corporation(,|$)') {
        throw "$Label is not signed by the trusted Microsoft toolchain publisher"
    }
    return $Item
}

function Assert-X64Pe([string] $Path, [string] $Label) {
    $Bytes = [IO.File]::ReadAllBytes($Path)
    if ($Bytes.Length -lt 64 -or $Bytes[0] -ne 0x4d -or $Bytes[1] -ne 0x5a) {
        throw "$Label is not a PE file"
    }
    $Offset = [BitConverter]::ToUInt32($Bytes, 0x3c)
    if ($Offset + 6 -gt $Bytes.Length -or
        $Bytes[$Offset] -ne 0x50 -or $Bytes[$Offset + 1] -ne 0x45 -or
        $Bytes[$Offset + 2] -ne 0 -or $Bytes[$Offset + 3] -ne 0 -or
        $Bytes[$Offset + 4] -ne 0x64 -or $Bytes[$Offset + 5] -ne 0x86) {
        throw "$Label is not Windows x64"
    }
}

function Assert-Archive([string] $Path) {
    $Item = Assert-RegularFile $Path "pinned llama.cpp source archive"
    if ($Item.Length -ne [long] $Policy.source.size -or
        (Get-LowerSha256 $Item.FullName) -ne $Policy.source.sha256) {
        throw "llama.cpp source archive did not match the pinned size and SHA-256"
    }
}

function Assert-CleanNativeBuildEnvironment {
    $InjectionVariables = @(
        "CL", "_CL_", "LINK", "_LINK_", "CC", "CXX", "RC", "ASM",
        "CFLAGS", "CXXFLAGS", "CPPFLAGS", "RCFLAGS", "ASMFLAGS", "LDFLAGS",
        "INCLUDE", "LIB", "LIBPATH",
        "CMAKE_TOOLCHAIN_FILE", "CMAKE_PROJECT_INCLUDE",
        "CMAKE_PROJECT_INCLUDE_BEFORE", "CMAKE_PROJECT_TOP_LEVEL_INCLUDES",
        "CMAKE_PREFIX_PATH", "CMAKE_PROGRAM_PATH", "CMAKE_MODULE_PATH",
        "CMAKE_GENERATOR", "CMAKE_GENERATOR_INSTANCE", "CMAKE_GENERATOR_PLATFORM",
        "CMAKE_GENERATOR_TOOLSET", "CMAKE_C_COMPILER", "CMAKE_CXX_COMPILER",
        "CMAKE_ASM_COMPILER", "CMAKE_C_COMPILER_LAUNCHER",
        "CMAKE_CXX_COMPILER_LAUNCHER", "CMAKE_ASM_COMPILER_LAUNCHER",
        "CMAKE_C_LINKER_LAUNCHER", "CMAKE_CXX_LINKER_LAUNCHER",
        "CMAKE_ASM_LINKER_LAUNCHER",
        "C_COMPILER_LAUNCHER", "CXX_COMPILER_LAUNCHER", "ASM_COMPILER_LAUNCHER",
        "C_LINKER_LAUNCHER", "CXX_LINKER_LAUNCHER", "ASM_LINKER_LAUNCHER",
        "CMAKE_LINKER", "CMAKE_AR", "CMAKE_RC_COMPILER", "CMAKE_MT",
        "CMAKE_MAKE_PROGRAM", "GIT_EXECUTABLE", "GIT_EXE"
    )
    foreach ($VariableName in $InjectionVariables) {
        $Value = [Environment]::GetEnvironmentVariable($VariableName, "Process")
        if (-not [string]::IsNullOrWhiteSpace($Value)) {
            throw "native build injection variable $VariableName must be unset"
        }
    }
}

function Apply-ReviewedSourcePatch([string] $SourceRoot) {
    $Patches = @($Policy.source_patches)
    if ($Patches.Count -ne 1 -or
        $Patches[0].id -ne "replace-unlicensed-bicubic-with-pillow-path" -or
        $Patches[0].path -ne "tools/mtmd/mtmd-image.cpp" -or
        $Patches[0].input_sha256 -ne "84d130afea62061871e8daef3fe8188415d4bcea0bcf9278955083700f951a65") {
        throw "the reviewed llama.cpp source patch policy changed"
    }

    $Patch = $Patches[0]
    $PatchPath = Join-Path $SourceRoot $Patch.path
    $PatchItem = Assert-RegularFile $PatchPath "reviewed llama.cpp source patch input"
    if ((Get-LowerSha256 $PatchItem.FullName) -ne $Patch.input_sha256) {
        throw "the reviewed llama.cpp source patch input changed"
    }

    $SourceText = [IO.File]::ReadAllText($PatchItem.FullName).Replace("`r`n", "`n")
    $StartMarker = "    static void resize_bicubic("
    $EndMarker = "`n    // Bicubic resize function using Pillow's ImagingResample algorithm"
    $Start = $SourceText.IndexOf($StartMarker, [StringComparison]::Ordinal)
    $End = $SourceText.IndexOf($EndMarker, [StringComparison]::Ordinal)
    if ($Start -lt 0 -or $End -le $Start -or
        $SourceText.LastIndexOf($StartMarker, [StringComparison]::Ordinal) -ne $Start -or
        $SourceText.LastIndexOf($EndMarker, [StringComparison]::Ordinal) -ne $End) {
        throw "the reviewed llama.cpp source patch markers are missing or ambiguous"
    }

    $Replacement = @'
    static void resize_bicubic(const clip_image_u8 & img, clip_image_u8 & dst, int target_width, int target_height) {
        if (!resize_bicubic_pillow(img, dst, target_width, target_height)) {
            throw std::runtime_error("Pillow-compatible bicubic resize failed");
        }
    }
'@
    $Replacement = $Replacement.Replace("`r`n", "`n")
    $PatchedText = $SourceText.Substring(0, $Start) + $Replacement + $SourceText.Substring($End)
    if ($PatchedText.Contains("yglukhov/bicubic-interpolation-image-processing")) {
        throw "the reviewed llama.cpp source patch did not remove the unlicensed implementation"
    }
    [IO.File]::WriteAllText(
        $PatchItem.FullName,
        $PatchedText,
        [Text.UTF8Encoding]::new($false)
    )
    $OutputHash = Get-LowerSha256 $PatchItem.FullName
    if ($OutputHash -ne $Patch.output_sha256) {
        throw "reviewed llama.cpp source patch produced SHA-256 $OutputHash instead of the pinned output"
    }
}

function Import-PinnedVcEnvironment([string] $Vcvars) {
    $EnvironmentScript = Join-Path $WorkRoot "vc-environment.cmd"
    @(
        "@echo off",
        "call `"$Vcvars`" x64 $($Policy.toolchain.windows_sdk_version) -vcvars_ver=14.44 >nul",
        "set"
    ) | Set-Content -LiteralPath $EnvironmentScript -Encoding Ascii
    $EnvironmentLines = & $CmdItem.FullName /d /q /c $EnvironmentScript
    if ($LASTEXITCODE -ne 0) {
        throw "Visual C++ environment initialization failed"
    }
    foreach ($Line in $EnvironmentLines) {
        $Separator = $Line.IndexOf('=')
        if ($Separator -gt 0) {
            [Environment]::SetEnvironmentVariable(
                $Line.Substring(0, $Separator),
                $Line.Substring($Separator + 1),
                "Process"
            )
        }
    }
    if ([string]::IsNullOrWhiteSpace($env:VCToolsVersion) -or
        -not $env:VCToolsVersion.TrimEnd('\').StartsWith(
            $Policy.toolchain.vc_tools_version_prefix,
            [StringComparison]::Ordinal
        )) {
        throw "the active VC tools are outside the reviewed 14.44 family"
    }
    if ([string]::IsNullOrWhiteSpace($env:WindowsSDKVersion) -or
        $env:WindowsSDKVersion.TrimEnd('\') -ne $Policy.toolchain.windows_sdk_version) {
        throw "the active Windows SDK does not match the pinned version"
    }
    [Environment]::SetEnvironmentVariable("COMSPEC", $CmdItem.FullName, "Process")
    if (-not [IO.Path]::GetFullPath($env:COMSPEC).Equals(
            [IO.Path]::GetFullPath($CmdItem.FullName),
            [StringComparison]::OrdinalIgnoreCase
        )) {
        throw "COMSPEC did not remain bound to the signed System32 command processor"
    }
}

function Assert-CmakeToolBinding(
    [string] $CacheText,
    [string] $VariableName,
    [string] $ExpectedPath
) {
    $Prefix = "$VariableName`:"
    $Matches = @($CacheText.Replace("`r`n", "`n").Split("`n") | Where-Object {
        $_.StartsWith($Prefix, [StringComparison]::Ordinal) -and $_.Contains('=')
    })
    if ($Matches.Count -ne 1) {
        throw "CMake cache has a missing or ambiguous $VariableName binding"
    }
    $Separator = $Matches[0].IndexOf('=')
    $ActualPath = $Matches[0].Substring($Separator + 1).Replace('/', '\')
    if (-not [IO.Path]::GetFullPath($ActualPath).Equals(
            [IO.Path]::GetFullPath($ExpectedPath),
            [StringComparison]::OrdinalIgnoreCase
        )) {
        throw "CMake cache changed the reviewed $VariableName binding"
    }
}

function Assert-CmakeCacheValue(
    [string] $CacheText,
    [string] $VariableName,
    [string] $ExpectedValue
) {
    $Prefix = "$VariableName`:"
    $Matches = @($CacheText.Replace("`r`n", "`n").Split("`n") | Where-Object {
        $_.StartsWith($Prefix, [StringComparison]::Ordinal) -and $_.Contains('=')
    })
    if ($Matches.Count -ne 1) {
        throw "CMake cache has a missing or ambiguous $VariableName value"
    }
    $Separator = $Matches[0].IndexOf('=')
    if ($Matches[0].Substring($Separator + 1) -cne $ExpectedValue) {
        throw "CMake cache changed the reviewed $VariableName value"
    }
}

function Get-ImportedDlls([string] $Dumpbin, [string] $Server) {
    $Output = & $Dumpbin /nologo /dependents $Server
    if ($LASTEXITCODE -ne 0) {
        throw "dumpbin could not inspect llama-server imports"
    }
    return @($Output | ForEach-Object {
        $Candidate = $_.Trim()
        if ($Candidate -match '^[A-Za-z0-9._-]+\.dll$') {
            $Candidate
        }
    } | Sort-Object -Unique)
}

function Invoke-VersionSmoke([string] $Server) {
    $StartInfo = [Diagnostics.ProcessStartInfo]::new()
    $StartInfo.FileName = $Server
    $StartInfo.Arguments = "--version"
    $StartInfo.UseShellExecute = $false
    $StartInfo.CreateNoWindow = $true
    $StartInfo.RedirectStandardOutput = $true
    $StartInfo.RedirectStandardError = $true
    $Process = [Diagnostics.Process]::new()
    $Process.StartInfo = $StartInfo
    try {
        if (-not $Process.Start()) {
            throw "llama-server --version could not start"
        }
        if (-not $Process.WaitForExit(10000)) {
            $Process.Kill()
            throw "llama-server --version did not finish within ten seconds"
        }
        $Output = ($Process.StandardOutput.ReadToEnd() + "`n" +
            $Process.StandardError.ReadToEnd()).Trim()
        $ExitCode = $Process.ExitCode
    } finally {
        $Process.Dispose()
    }
    if ($ExitCode -ne 0 -or
        $Output -notmatch "version: 9946 \($($Policy.commit)\)" -or
        $Output -notmatch "built with MSVC $($Policy.toolchain.msvc_compiler_version_prefix)") {
        throw "llama-server version smoke test did not report the pinned source and compiler family"
    }
    return $Output
}

Assert-CleanNativeBuildEnvironment
$CmdItem = Assert-MicrosoftSignedTool `
    (Join-Path $env:SystemRoot "System32\cmd.exe") `
    "Windows command processor"
$CurlItem = Assert-MicrosoftSignedTool `
    (Join-Path $env:SystemRoot "System32\curl.exe") `
    "Windows curl downloader"

if ([Environment]::GetEnvironmentVariable($InternalSingleBuildEnv, "Process") -ne "1") {
    $PowerShell = Join-Path $PSHOME "powershell.exe"
    $null = Assert-MicrosoftSignedTool $PowerShell "Windows PowerShell build orchestrator"
    $PreviousInternalMode = [Environment]::GetEnvironmentVariable(
        $InternalSingleBuildEnv,
        "Process"
    )
    try {
        [Environment]::SetEnvironmentVariable($InternalSingleBuildEnv, "1", "Process")

        & $PowerShell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $PSCommandPath
        if ($LASTEXITCODE -ne 0) {
            throw "the first isolated llama.cpp build failed"
        }
        Assert-NoReparseAncestor $Destination "first llama.cpp Windows runtime"
        $FirstServer = Assert-RegularFile `
            (Join-Path $Destination "llama-server.exe") `
            "first isolated llama-server.exe"
        [long] $FirstSize = $FirstServer.Length
        $FirstHash = Get-LowerSha256 $FirstServer.FullName

        & $PowerShell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $PSCommandPath
        if ($LASTEXITCODE -ne 0) {
            throw "the second isolated llama.cpp build failed"
        }
        Assert-NoReparseAncestor $Destination "second llama.cpp Windows runtime"
        $SecondServer = Assert-RegularFile `
            (Join-Path $Destination "llama-server.exe") `
            "second isolated llama-server.exe"
        [long] $SecondSize = $SecondServer.Length
        $SecondHash = Get-LowerSha256 $SecondServer.FullName
        if ($FirstSize -ne $SecondSize -or $FirstHash -ne $SecondHash) {
            throw "the two isolated llama.cpp builds were not byte-reproducible"
        }

        $ManifestPath = Join-Path $Destination $ManifestName
        $ManifestItem = Assert-RegularFile `
            $ManifestPath `
            "second isolated llama.cpp build manifest"
        try {
            $Manifest = Get-Content -LiteralPath $ManifestItem.FullName -Raw | ConvertFrom-Json
        } catch {
            throw "the second isolated llama.cpp build manifest is invalid JSON"
        }
        $RuntimeFiles = @($Manifest.runtime.files)
        if ($RuntimeFiles.Count -ne 1 -or
            $RuntimeFiles[0].path -ne "llama-server.exe" -or
            [long] $RuntimeFiles[0].size -ne $SecondSize -or
            $RuntimeFiles[0].sha256 -ne $SecondHash) {
            throw "the second isolated build manifest does not authenticate its runtime"
        }
        $Manifest | Add-Member -NotePropertyName reproducibility -NotePropertyValue ([ordered]@{
            build_count = 2
            isolated_work_roots = $true
            matching_outputs = $true
            builds = @(
                [ordered]@{ ordinal = 1; size = $FirstSize; sha256 = $FirstHash },
                [ordered]@{ ordinal = 2; size = $SecondSize; sha256 = $SecondHash }
            )
        })
        $ManifestJson = ($Manifest | ConvertTo-Json -Depth 10).Replace("`r`n", "`n") + "`n"
        $ManifestTemp = Join-Path $Destination "BUILD-MANIFEST.reproducibility.tmp"
        $ManifestBackup = Join-Path $Destination "BUILD-MANIFEST.reproducibility.backup"
        Assert-NoReparseAncestor $ManifestTemp "llama.cpp reproducibility receipt staging"
        Assert-NoReparseAncestor $ManifestBackup "llama.cpp reproducibility receipt backup"
        [IO.File]::WriteAllText(
            $ManifestTemp,
            $ManifestJson,
            [Text.UTF8Encoding]::new($false)
        )
        Set-WindowsAtomicFileReplacement `
            $ManifestTemp `
            $ManifestItem.FullName `
            $ManifestBackup `
            "llama.cpp reproducibility receipt"
        $env:AIRWIKI_WINDOWS_LLAMA_SERVER_SHA256 = $SecondHash
        Write-Host "Verified two byte-reproducible isolated llama.cpp builds"
        Write-Host "llama-server.exe SHA-256: $SecondHash"
    } finally {
        [Environment]::SetEnvironmentVariable(
            $InternalSingleBuildEnv,
            $PreviousInternalMode,
            "Process"
        )
    }
    return
}

if ($Policy.schema_version -ne 1 -or $Policy.component -ne "llama.cpp" -or
    $Policy.tag -ne "b9946" -or
    $Policy.commit -ne "fb30ba9a6c5b4674174d06aed14794832ab33278" -or
    $Policy.build.openmp -ne $false -or $Policy.build.shared_libraries -ne $false -or
    $Policy.build.msvc_runtime -ne "MultiThreaded") {
    throw "the llama.cpp Windows build policy is incompatible or unsafe"
}

$AllowedWorkRoot = [IO.Path]::GetFullPath($WorkParent)
$AllowedDestinationRoot = [IO.Path]::GetFullPath((Join-Path $Root "resources\llama"))
if (-not ([IO.Path]::GetFullPath($WorkRoot).StartsWith(
        $AllowedWorkRoot + [IO.Path]::DirectorySeparatorChar,
        [StringComparison]::OrdinalIgnoreCase
    )) -or
    -not ([IO.Path]::GetFullPath($Destination).StartsWith(
        $AllowedDestinationRoot + [IO.Path]::DirectorySeparatorChar,
        [StringComparison]::OrdinalIgnoreCase
    ))) {
    throw "refusing to build or replace llama.cpp outside the fixed workspace roots"
}
Assert-NoReparseAncestor $Root "workspace root"
Assert-NoReparseAncestor $Cache "llama.cpp source cache"
Assert-NoReparseAncestor $Archive "llama.cpp source archive"
Assert-NoReparseAncestor $WorkParent "llama.cpp source-build staging parent"
Assert-NoReparseAncestor $WorkRoot "llama.cpp source-build staging"
Assert-NoReparseAncestor $Destination "llama.cpp Windows runtime destination"
New-Item -ItemType Directory -Path $AllowedDestinationRoot -Force | Out-Null
Assert-NoReparseAncestor $AllowedDestinationRoot "llama.cpp Windows runtime parent"

New-Item -ItemType Directory -Path $Cache -Force | Out-Null
$ArchiveValid = $false
if (Test-Path -LiteralPath $Archive -PathType Leaf) {
    $ArchiveItem = Get-Item -LiteralPath $Archive -Force
    $ArchiveValid = $ArchiveItem.Length -eq [long] $Policy.source.size -and
        (Get-LowerSha256 $ArchiveItem.FullName) -eq $Policy.source.sha256
}
if (-not $ArchiveValid) {
    & $CurlItem.FullName -fL --retry 3 -C - -o $Archive $Policy.source.url
    if ($LASTEXITCODE -ne 0) {
        Remove-Item -LiteralPath $Archive -Force -ErrorAction SilentlyContinue
        & $CurlItem.FullName -fL --retry 3 -o $Archive $Policy.source.url
        if ($LASTEXITCODE -ne 0) {
            throw "failed to download the pinned llama.cpp source archive"
        }
    }
}
try {
    Assert-Archive $Archive
} catch {
    Remove-Item -LiteralPath $Archive -Force -ErrorAction SilentlyContinue
    & $CurlItem.FullName -fL --retry 3 -o $Archive $Policy.source.url
    if ($LASTEXITCODE -ne 0) {
        throw "failed to redownload the pinned llama.cpp source archive"
    }
    Assert-Archive $Archive
}

New-Item -ItemType Directory -Path $ExtractRoot, $RuntimeStage -Force | Out-Null
$Tar = Join-Path $env:SystemRoot "System32\tar.exe"
$TarItem = Assert-MicrosoftSignedTool $Tar "Windows archive extractor"
& $TarItem.FullName -xf $Archive -C $ExtractRoot
if ($LASTEXITCODE -ne 0) {
    throw "the signed Windows archive extractor could not unpack llama.cpp"
}
$ArchiveRoots = @(Get-ChildItem -LiteralPath $ExtractRoot -Force)
if ($ArchiveRoots.Count -ne 1 -or -not $ArchiveRoots[0].PSIsContainer -or
    $ArchiveRoots[0].Name -ne $Policy.source.root_directory) {
    throw "the pinned source archive has an unexpected root layout"
}
foreach ($Entry in Get-ChildItem -LiteralPath $Source -Force -Recurse) {
    if (($Entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "the pinned source archive produced a reparse point"
    }
}
foreach ($LegalSource in $Policy.legal_sources) {
    $LegalPath = Join-Path $Source $LegalSource.path
    $LegalItem = Assert-RegularFile $LegalPath "llama.cpp legal source $($LegalSource.path)"
    if ((Get-LowerSha256 $LegalItem.FullName) -ne $LegalSource.sha256) {
        throw "llama.cpp legal source $($LegalSource.path) did not match the reviewed bytes"
    }
}
Apply-ReviewedSourcePatch $Source

$Vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
$VswhereItem = Assert-MicrosoftSignedTool $Vswhere "Visual Studio locator"
$InstallationJson = & $VswhereItem.FullName -latest -products * `
    -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -format json
if ($LASTEXITCODE -ne 0) {
    throw "Visual Studio locator failed"
}
$Installations = @($InstallationJson | ConvertFrom-Json)
if ($Installations.Count -ne 1 -or
    -not $Installations[0].installationVersion.StartsWith(
        $Policy.toolchain.visual_studio_installation_version_prefix,
        [StringComparison]::Ordinal
    )) {
    throw "Visual Studio 2022 17.14 with the x64 C++ tools is required"
}
$Vs = $Installations[0].installationPath
$Cmake = Join-Path $Vs "Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe"
$Ninja = Join-Path $Vs "Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja\ninja.exe"
$Vcvars = Join-Path $Vs "VC\Auxiliary\Build\vcvarsall.bat"
Import-PinnedVcEnvironment $Vcvars
$VcTools = $env:VCToolsInstallDir.TrimEnd('\')
$Cl = Join-Path $VcTools "bin\Hostx64\x64\cl.exe"
$Link = Join-Path $VcTools "bin\Hostx64\x64\link.exe"
$Lib = Join-Path $VcTools "bin\Hostx64\x64\lib.exe"
$Dumpbin = Join-Path $VcTools "bin\Hostx64\x64\dumpbin.exe"
$Rc = Join-Path $env:WindowsSdkDir "bin\$($Policy.toolchain.windows_sdk_version)\x64\rc.exe"
$Mt = Join-Path $env:WindowsSdkDir "bin\$($Policy.toolchain.windows_sdk_version)\x64\mt.exe"
$CmakeNinja = $Ninja.Replace('\', '/')
$CmakeCl = $Cl.Replace('\', '/')
$CmakeLink = $Link.Replace('\', '/')
$CmakeLib = $Lib.Replace('\', '/')
$CmakeRc = $Rc.Replace('\', '/')
$CmakeMt = $Mt.Replace('\', '/')
$ToolItems = [ordered]@{
    cmd = $CmdItem
    curl = $CurlItem
    tar = $TarItem
    cmake = Assert-MicrosoftSignedTool $Cmake "Visual Studio CMake"
    ninja = Assert-MicrosoftSignedTool $Ninja "Visual Studio Ninja"
    cl = Assert-MicrosoftSignedTool $Cl "MSVC compiler"
    link = Assert-MicrosoftSignedTool $Link "MSVC linker"
    lib = Assert-MicrosoftSignedTool $Lib "MSVC librarian"
    rc = Assert-MicrosoftSignedTool $Rc "Windows SDK resource compiler"
    mt = Assert-MicrosoftSignedTool $Mt "Windows SDK manifest tool"
    dumpbin = Assert-MicrosoftSignedTool $Dumpbin "MSVC import inspector"
}
$CmakeVersion = (Invoke-WindowsToolVersionLine `
        $Cmake `
        @("--version") `
        "Visual Studio CMake") -replace '^cmake version ', ''
$NinjaVersion = Invoke-WindowsToolVersionLine `
    $Ninja `
    @("--version") `
    "Visual Studio Ninja"
$CompilerVersion = $ToolItems.cl.VersionInfo.FileVersion
if ($CmakeVersion -ne $Policy.toolchain.cmake_version -or
    $Policy.toolchain.ninja_versions -notcontains $NinjaVersion -or
    -not $CompilerVersion.StartsWith(
        $Policy.toolchain.msvc_compiler_version_prefix,
        [StringComparison]::Ordinal
    )) {
    throw "the native build tools are outside the reviewed Windows toolchain policy"
}

$env:SOURCE_DATE_EPOCH = [string] $Policy.source_date_epoch
$CompilerFlags = @($Policy.build.compiler_flags) + @(
    "/pathmap:$Source=llama.cpp",
    "/pathmap:$Build=build"
)
$LinkerFlags = @($Policy.build.linker_flags)
$ConfigureArguments = @(
    "-S", $Source,
    "-B", $Build,
    "-G", "Ninja",
    "-DCMAKE_MAKE_PROGRAM=$CmakeNinja",
    "-DCMAKE_C_COMPILER=$CmakeCl",
    "-DCMAKE_CXX_COMPILER=$CmakeCl",
    "-DCMAKE_ASM_COMPILER=$CmakeCl",
    "-DCMAKE_LINKER=$CmakeLink",
    "-DCMAKE_AR=$CmakeLib",
    "-DCMAKE_RC_COMPILER=$CmakeRc",
    "-DCMAKE_MT=$CmakeMt",
    "-DCMAKE_BUILD_TYPE=Release",
    "-DCMAKE_MSVC_RUNTIME_LIBRARY=$($Policy.build.msvc_runtime)",
    "-DCMAKE_C_FLAGS_RELEASE=$([string]::Join(' ', $CompilerFlags))",
    "-DCMAKE_CXX_FLAGS_RELEASE=$([string]::Join(' ', $CompilerFlags + @('/EHsc')))",
    "-DCMAKE_EXE_LINKER_FLAGS_RELEASE=$([string]::Join(' ', $LinkerFlags))",
    "-DCMAKE_SHARED_LINKER_FLAGS_RELEASE=$([string]::Join(' ', $LinkerFlags))"
) + @($Policy.build.cmake_definitions)

& $Cmake @ConfigureArguments
if ($LASTEXITCODE -ne 0) {
    throw "llama.cpp CMake configuration failed"
}
$CacheText = Get-Content -LiteralPath (Join-Path $Build "CMakeCache.txt") -Raw
foreach ($RequiredCacheEntry in @(
    "BUILD_SHARED_LIBS:BOOL=OFF",
    "GGML_BACKEND_DL:BOOL=OFF",
    "GGML_OPENMP:BOOL=OFF",
    "GGML_NATIVE:BOOL=OFF",
    "LLAMA_OPENSSL:BOOL=OFF",
    "CMAKE_MSVC_RUNTIME_LIBRARY:UNINITIALIZED=MultiThreaded"
)) {
    if ($CacheText.IndexOf($RequiredCacheEntry, [StringComparison]::Ordinal) -lt 0) {
        throw "configured llama.cpp cache is missing invariant $RequiredCacheEntry"
    }
}
foreach ($ToolBinding in @(
    @("CMAKE_C_COMPILER", $Cl),
    @("CMAKE_CXX_COMPILER", $Cl),
    @("CMAKE_ASM_COMPILER", $Cl),
    @("CMAKE_LINKER", $Link),
    @("CMAKE_AR", $Lib),
    @("CMAKE_RC_COMPILER", $Rc),
    @("CMAKE_MT", $Mt),
    @("CMAKE_MAKE_PROGRAM", $Ninja)
)) {
    Assert-CmakeToolBinding $CacheText $ToolBinding[0] $ToolBinding[1]
}
foreach ($BlockedGitBinding in @(
    @("CMAKE_DISABLE_FIND_PACKAGE_Git", "TRUE"),
    @("GIT_EXECUTABLE", "OFF"),
    @("GIT_EXE", "OFF")
)) {
    Assert-CmakeCacheValue $CacheText $BlockedGitBinding[0] $BlockedGitBinding[1]
}
foreach ($BlockedLauncher in @(
    "CMAKE_C_COMPILER_LAUNCHER",
    "CMAKE_CXX_COMPILER_LAUNCHER",
    "CMAKE_ASM_COMPILER_LAUNCHER",
    "CMAKE_C_LINKER_LAUNCHER",
    "CMAKE_CXX_LINKER_LAUNCHER",
    "CMAKE_ASM_LINKER_LAUNCHER"
)) {
    Assert-CmakeCacheValue $CacheText $BlockedLauncher ""
}
& $Cmake --build $Build --target llama-server --config Release --parallel 2
if ($LASTEXITCODE -ne 0) {
    throw "llama.cpp server build failed"
}
if (@(Get-ChildItem -LiteralPath $Build -Recurse -File -Filter *.dll).Count -ne 0) {
    throw "the static llama.cpp build unexpectedly produced a DLL"
}
$Server = Join-Path $Build "bin\llama-server.exe"
$ServerItem = Assert-RegularFile $Server "built llama-server.exe"
Assert-X64Pe $ServerItem.FullName "built llama-server.exe"
$ServerHash = Get-LowerSha256 $ServerItem.FullName
$Imports = @(Get-ImportedDlls $Dumpbin $ServerItem.FullName)
$ExpectedImports = @($Policy.target.imports | Sort-Object -Unique)
if (@(Compare-Object $ExpectedImports $Imports).Count -ne 0 -or
    @($Imports | Where-Object { $_ -match '(?i)(omp|vcruntime|msvcp|ucrt)' }).Count -ne 0) {
    throw "llama-server imports are outside the reviewed system-DLL allowlist"
}
$VersionOutput = Invoke-VersionSmoke $ServerItem.FullName
$RunnerImageVersion = if ($null -eq $env:ImageVersion) {
    "local"
} else {
    $env:ImageVersion
}

$ToolManifest = [ordered]@{}
foreach ($ToolName in $ToolItems.Keys) {
    $Tool = $ToolItems[$ToolName]
    $ToolVersion = [string] $Tool.VersionInfo.FileVersion
    if ([string]::IsNullOrWhiteSpace($ToolVersion)) {
        if ($ToolName -ne "ninja") {
            throw "the $ToolName tool receipt lacks a version"
        }
        $ToolVersion = $NinjaVersion
    }
    $ToolManifest[$ToolName] = [ordered]@{
        file_version = $ToolVersion
        size = $Tool.Length
        sha256 = Get-LowerSha256 $Tool.FullName
    }
}
$Manifest = [ordered]@{
    schema_version = 1
    component = $Policy.component
    tag = $Policy.tag
    commit = $Policy.commit
    policy_sha256 = Get-LowerSha256 $PolicyPath
    source = [ordered]@{
        url = $Policy.source.url
        size = [long] $Policy.source.size
        sha256 = $Policy.source.sha256
    }
    target = [ordered]@{
        triple = $Policy.target.triple
        minimum_cpu = $Policy.target.minimum_cpu
    }
    toolchain = [ordered]@{
        visual_studio_installation_version = $Installations[0].installationVersion
        vc_tools_version = $env:VCToolsVersion.TrimEnd('\')
        windows_sdk_version = $env:WindowsSDKVersion.TrimEnd('\')
        cmake_version = $CmakeVersion
        ninja_version = $NinjaVersion
        runner_image_version = $RunnerImageVersion
        files = $ToolManifest
    }
    build = [ordered]@{
        source_date_epoch = [long] $Policy.source_date_epoch
        msvc_runtime = $Policy.build.msvc_runtime
        shared_libraries = $false
        openmp = $false
        compiler_flags = @($Policy.build.compiler_flags) + @(
            "/pathmap:<SOURCE>=llama.cpp",
            "/pathmap:<BUILD>=build"
        )
        linker_flags = @($Policy.build.linker_flags)
        cmake_definitions = @($Policy.build.cmake_definitions)
        source_patches = @($Policy.source_patches)
        tool_bindings = [ordered]@{
            c_compiler = "cl"
            cxx_compiler = "cl"
            asm_compiler = "cl"
            linker = "link"
            librarian = "lib"
            resource_compiler = "rc"
            manifest_tool = "mt"
            build_program = "ninja"
        }
    }
    runtime = [ordered]@{
        files = @([ordered]@{
            path = "llama-server.exe"
            size = $ServerItem.Length
            sha256 = $ServerHash
        })
        imports = $Imports
        version_output = $VersionOutput
    }
}
$ManifestJson = ($Manifest | ConvertTo-Json -Depth 10).Replace("`r`n", "`n") + "`n"
[IO.File]::Copy($ServerItem.FullName, (Join-Path $RuntimeStage "llama-server.exe"), $false)
[IO.File]::WriteAllText(
    (Join-Path $RuntimeStage $ManifestName),
    $ManifestJson,
    [Text.UTF8Encoding]::new($false)
)

if (Test-Path -LiteralPath $Destination) {
    Remove-DirectoryWithRetry $Destination "existing llama.cpp Windows runtime"
}
Assert-NoReparseAncestor $RuntimeStage "verified llama.cpp runtime staging"
Assert-NoReparseAncestor $Destination "llama.cpp Windows runtime destination"
Move-Item -LiteralPath $RuntimeStage -Destination $Destination
$env:AIRWIKI_WINDOWS_LLAMA_SERVER_SHA256 = $ServerHash
try {
    Remove-DirectoryWithRetry $WorkRoot "llama.cpp source-build staging"
} catch {
    Write-Warning "llama.cpp source-build staging remains under target and can be removed later"
}
Write-Host "Built verified llama.cpp $($Policy.tag) Windows runtime at $Destination"
Write-Host "llama-server.exe SHA-256: $ServerHash"
