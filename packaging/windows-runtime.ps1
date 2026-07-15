$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$script:WindowsRuntimeMaxEntries = 4096
$script:WindowsRuntimeMaxBytes = 2GB

function Assert-WindowsAbsolutePath([string] $Path, [string] $Label) {
    $IsDriveAbsolute = -not [string]::IsNullOrWhiteSpace($Path) -and
        $Path -match '^[A-Za-z]:[\\/]'
    $IsUncAbsolute = -not [string]::IsNullOrWhiteSpace($Path) -and
        $Path -match '^\\\\[^\\/]+[\\/][^\\/]+(?:[\\/]|$)'
    if (-not $IsDriveAbsolute -and -not $IsUncAbsolute) {
        throw "$Label must be an absolute path"
    }
    foreach ($Segment in [regex]::Split($Path, '[\\/]+')) {
        if ($Segment -eq "." -or $Segment -eq "..") {
            throw "$Label contains a traversal segment"
        }
    }
}

function Test-WindowsOrdinalSequenceEqual([object[]] $Expected, [object[]] $Actual) {
    $ExpectedItems = @($Expected)
    $ActualItems = @($Actual)
    if ($ExpectedItems.Count -ne $ActualItems.Count) {
        return $false
    }
    for ($Index = 0; $Index -lt $ExpectedItems.Count; $Index += 1) {
        if ([string] $ExpectedItems[$Index] -cne [string] $ActualItems[$Index]) {
            return $false
        }
    }
    return $true
}

function Assert-NoWindowsReparseAncestor([string] $Path, [string] $Label) {
    $CurrentPath = [IO.Path]::GetFullPath($Path)
    while ($null -ne $CurrentPath) {
        $Item = Get-Item -LiteralPath $CurrentPath -Force -ErrorAction Stop
        if (($Item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "$Label or one of its ancestors is a reparse point"
        }
        $Parent = [IO.Directory]::GetParent($CurrentPath)
        if ($null -eq $Parent) {
            $CurrentPath = $null
        } else {
            $CurrentPath = $Parent.FullName
        }
    }
}

function Get-VerifiedWindowsRegularFile([string] $Path, [string] $Label) {
    Assert-WindowsAbsolutePath $Path $Label
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "$Label is missing or is not a regular file"
    }
    $Item = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
    Assert-NoWindowsReparseAncestor $Item.FullName $Label
    return $Item.FullName
}

function Invoke-WindowsToolVersionLine(
    [string] $Path,
    [string[]] $Arguments,
    [string] $Label
) {
    $ResolvedPath = Get-VerifiedWindowsRegularFile $Path $Label
    $Output = @(& $ResolvedPath @Arguments)
    $ExitCode = $LASTEXITCODE
    if ($ExitCode -ne 0) {
        throw "$Label version command failed with exit code $ExitCode"
    }
    if ($Output.Count -lt 1 -or [string]::IsNullOrWhiteSpace([string] $Output[0])) {
        throw "$Label version command returned no version"
    }
    return ([string] $Output[0]).Trim()
}

function Set-WindowsAtomicFileReplacement(
    [string] $StagedPath,
    [string] $DestinationPath,
    [string] $BackupPath,
    [string] $Label
) {
    $ResolvedStaged = Get-VerifiedWindowsRegularFile $StagedPath "$Label staging file"
    $ResolvedDestination = Get-VerifiedWindowsRegularFile `
        $DestinationPath `
        "$Label destination"
    Assert-WindowsAbsolutePath $BackupPath "$Label backup"
    $BackupParent = Split-Path -Parent $BackupPath
    Assert-NoWindowsReparseAncestor $BackupParent "$Label backup parent"

    $DistinctPaths = @(
        @($ResolvedStaged, $ResolvedDestination, $BackupPath) |
            ForEach-Object { [IO.Path]::GetFullPath($_).ToLowerInvariant() } |
            Sort-Object -Unique
    )
    if ($DistinctPaths.Count -ne 3) {
        throw "$Label staging, destination, and backup paths must be distinct"
    }

    $StagedItem = Get-Item -LiteralPath $ResolvedStaged -Force -ErrorAction Stop
    [long] $ExpectedLength = $StagedItem.Length
    $ExpectedHash = (Get-FileHash -LiteralPath $ResolvedStaged -Algorithm SHA256).Hash

    if (Test-Path -LiteralPath $BackupPath) {
        $null = Get-VerifiedWindowsRegularFile $BackupPath "$Label interrupted backup"
        $CurrentItem = Get-Item -LiteralPath $ResolvedDestination -Force -ErrorAction Stop
        $CurrentHash = (Get-FileHash -LiteralPath $ResolvedDestination -Algorithm SHA256).Hash
        if ($CurrentItem.Length -ne $ExpectedLength -or $CurrentHash -cne $ExpectedHash) {
            throw "$Label has an ambiguous interrupted replacement"
        }

        [IO.File]::Delete($ResolvedStaged)
        if (Test-Path -LiteralPath $ResolvedStaged) {
            throw "$Label committed staging file was not removed"
        }
        [IO.File]::Delete($BackupPath)
        if (Test-Path -LiteralPath $BackupPath) {
            throw "$Label interrupted backup was not removed"
        }
        return
    }

    [IO.File]::Replace($ResolvedStaged, $ResolvedDestination, $BackupPath)
    $null = Get-VerifiedWindowsRegularFile $BackupPath "$Label replacement backup"
    $CommittedItem = Get-Item -LiteralPath $ResolvedDestination -Force -ErrorAction Stop
    $CommittedHash = (Get-FileHash -LiteralPath $ResolvedDestination -Algorithm SHA256).Hash
    if ($CommittedItem.Length -ne $ExpectedLength -or $CommittedHash -cne $ExpectedHash) {
        throw "$Label replacement did not commit the staged bytes"
    }
    [IO.File]::Delete($BackupPath)
    if (Test-Path -LiteralPath $BackupPath) {
        throw "$Label replacement backup was not removed"
    }
}

function Initialize-WindowsManifestResourceReader {
    if ($null -ne ("AirWiki.WindowsManifestResourceReader" -as [type])) {
        return
    }

    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;

namespace AirWiki
{
    public sealed class ManifestResourceVariant
    {
        public ushort Language { get; private set; }
        public byte[] Bytes { get; private set; }

        internal ManifestResourceVariant(ushort language, byte[] bytes)
        {
            Language = language;
            Bytes = bytes;
        }
    }

    public static class WindowsManifestResourceReader
    {
        private const int MaximumLanguageVariants = 16;
        private const uint LoadLibraryAsImageResource = 0x00000020;
        private const uint LoadLibraryAsDataFileExclusive = 0x00000040;
        private static readonly IntPtr PrimaryManifest = new IntPtr(1);
        private static readonly IntPtr ManifestResourceType = new IntPtr(24);

        [UnmanagedFunctionPointer(CallingConvention.Winapi)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private delegate bool EnumResourceLanguagesCallback(
            IntPtr module,
            IntPtr resourceType,
            IntPtr name,
            ushort language,
            IntPtr parameter
        );

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern IntPtr LoadLibraryExW(
            string fileName,
            IntPtr reserved,
            uint flags
        );

        [DllImport("kernel32.dll", EntryPoint = "EnumResourceLanguagesW", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool EnumResourceLanguages(
            IntPtr module,
            IntPtr resourceType,
            IntPtr name,
            EnumResourceLanguagesCallback callback,
            IntPtr parameter
        );

        [DllImport("kernel32.dll", EntryPoint = "FindResourceExW", SetLastError = true)]
        private static extern IntPtr FindResourceEx(
            IntPtr module,
            IntPtr resourceType,
            IntPtr name,
            ushort language
        );

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern IntPtr LoadResource(IntPtr module, IntPtr resource);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern uint SizeofResource(IntPtr module, IntPtr resource);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern IntPtr LockResource(IntPtr resourceData);

        [DllImport("kernel32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool FreeLibrary(IntPtr module);

        private static Win32Exception NativeError(string operation)
        {
            return new Win32Exception(Marshal.GetLastWin32Error(), operation);
        }

        private static byte[] ReadManifestVariant(
            IntPtr module,
            ushort language
        )
        {
            IntPtr resource = FindResourceEx(
                module,
                ManifestResourceType,
                PrimaryManifest,
                language
            );
            if (resource == IntPtr.Zero)
            {
                throw NativeError("FindResourceExW failed");
            }

            uint size = SizeofResource(module, resource);
            if (size == 0 || size > 1024 * 1024)
            {
                throw new InvalidOperationException("manifest resource has an invalid size");
            }

            IntPtr resourceData = LoadResource(module, resource);
            if (resourceData == IntPtr.Zero)
            {
                throw NativeError("LoadResource failed");
            }
            IntPtr bytes = LockResource(resourceData);
            if (bytes == IntPtr.Zero)
            {
                throw NativeError("LockResource failed");
            }

            byte[] result = new byte[checked((int)size)];
            Marshal.Copy(bytes, result, 0, result.Length);
            return result;
        }

        public static ManifestResourceVariant[] ReadPrimaryManifestVariants(string path)
        {
            uint flags = LoadLibraryAsImageResource |
                LoadLibraryAsDataFileExclusive;
            IntPtr module = LoadLibraryExW(path, IntPtr.Zero, flags);
            if (module == IntPtr.Zero)
            {
                throw NativeError("LoadLibraryExW failed");
            }

            try
            {
                var languages = new System.Collections.Generic.List<ushort>();
                bool overflow = false;
                EnumResourceLanguagesCallback callback = delegate(
                    IntPtr ignoredModule,
                    IntPtr ignoredType,
                    IntPtr ignoredName,
                    ushort language,
                    IntPtr ignoredParameter)
                {
                    if (languages.Count >= MaximumLanguageVariants)
                    {
                        overflow = true;
                        return false;
                    }
                    languages.Add(language);
                    return true;
                };
                bool enumerated = EnumResourceLanguages(
                    module,
                    ManifestResourceType,
                    PrimaryManifest,
                    callback,
                    IntPtr.Zero
                );
                GC.KeepAlive(callback);
                if (overflow)
                {
                    throw new InvalidOperationException("manifest has too many language variants");
                }
                if (!enumerated || languages.Count == 0)
                {
                    throw NativeError("EnumResourceLanguagesW failed");
                }

                var seen = new System.Collections.Generic.HashSet<ushort>();
                var variants = new System.Collections.Generic.List<ManifestResourceVariant>();
                foreach (ushort language in languages)
                {
                    if (!seen.Add(language))
                    {
                        throw new InvalidOperationException("manifest has a duplicate language variant");
                    }
                    variants.Add(new ManifestResourceVariant(
                        language,
                        ReadManifestVariant(module, language)
                    ));
                }
                return variants.ToArray();
            }
            finally
            {
                FreeLibrary(module);
            }
        }
    }
}
'@
}

function Assert-WindowsFirewallHelperManifestVariants(
    [object[]] $Variants,
    [string] $Label
) {
    if ($null -eq $Variants -or $Variants.Count -ne 1) {
        throw "$Label must contain exactly one application manifest language variant"
    }

    $Variant = $Variants[0]
    if ($null -eq $Variant -or $null -eq $Variant.Bytes) {
        throw "$Label contains an invalid application manifest language variant"
    }
    [byte[]] $Bytes = $Variant.Bytes
    Assert-WindowsFirewallHelperManifestBytes $Bytes $Label
}

function Assert-WindowsFirewallHelperManifestBytes(
    [byte[]] $Bytes,
    [string] $Label
) {
    if ($null -eq $Bytes -or $Bytes.Length -eq 0 -or $Bytes.Length -gt 1MB) {
        throw "$Label has a missing or oversized embedded application manifest"
    }

    $Settings = [Xml.XmlReaderSettings]::new()
    $Settings.DtdProcessing = [Xml.DtdProcessing]::Prohibit
    $Settings.XmlResolver = $null
    $Settings.MaxCharactersInDocument = 1MB
    $Stream = [IO.MemoryStream]::new($Bytes, $false)
    $Reader = $null
    try {
        $Reader = [Xml.XmlReader]::Create($Stream, $Settings)
        $Document = [Xml.XmlDocument]::new()
        $Document.XmlResolver = $null
        $Document.Load($Reader)
    } catch {
        throw "$Label has an invalid embedded application manifest"
    } finally {
        if ($null -ne $Reader) { $Reader.Dispose() }
        $Stream.Dispose()
    }

    $Namespaces = [Xml.XmlNamespaceManager]::new($Document.NameTable)
    $Namespaces.AddNamespace("asmv1", "urn:schemas-microsoft-com:asm.v1")
    $Namespaces.AddNamespace("asmv3", "urn:schemas-microsoft-com:asm.v3")
    $ExpectedNodes = $Document.SelectNodes(
        "/asmv1:assembly/asmv3:trustInfo/asmv3:security/" +
        "asmv3:requestedPrivileges/asmv3:requestedExecutionLevel",
        $Namespaces
    )
    $AllExecutionLevels = $Document.SelectNodes("//*[local-name()='requestedExecutionLevel']")
    if ($ExpectedNodes.Count -ne 1 -or $AllExecutionLevels.Count -ne 1) {
        throw "$Label must contain exactly one requestedExecutionLevel in trustInfo"
    }

    $ExecutionLevel = $ExpectedNodes[0]
    if ($ExecutionLevel.GetAttribute("level") -cne "requireAdministrator") {
        throw "$Label must request requireAdministrator"
    }
    if ($ExecutionLevel.GetAttribute("uiAccess") -cne "false") {
        throw "$Label must set uiAccess=false"
    }
}

function Assert-WindowsFirewallHelperManifest([string] $Path, [string] $Label) {
    $Helper = Get-VerifiedWindowsRegularFile $Path $Label
    Initialize-WindowsManifestResourceReader
    try {
        $ManifestVariants = @(
            [AirWiki.WindowsManifestResourceReader]::ReadPrimaryManifestVariants($Helper)
        )
    } catch {
        throw "$Label embedded application manifest could not be read"
    }
    Assert-WindowsFirewallHelperManifestVariants $ManifestVariants $Label
}

function Get-WindowsRuntimeRelativePath(
    [string] $RootPrefix,
    [string] $EntryPath,
    [string] $Label
) {
    $FullPath = [IO.Path]::GetFullPath($EntryPath)
    if (-not $FullPath.StartsWith($RootPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "$Label runtime entry escaped its root"
    }
    $Relative = $FullPath.Substring($RootPrefix.Length)
    if ([string]::IsNullOrWhiteSpace($Relative) -or
        [IO.Path]::IsPathRooted($Relative)) {
        throw "$Label runtime tree contains an invalid relative path"
    }
    $Segments = @([regex]::Split($Relative, '[\\/]+'))
    foreach ($Segment in $Segments) {
        if ([string]::IsNullOrWhiteSpace($Segment) -or
            $Segment -eq "." -or $Segment -eq "..") {
            throw "$Label runtime tree contains an invalid relative path"
        }
    }
    return [string]::Join("/", $Segments)
}

function Get-VerifiedRuntimeTree([string] $Root, [string] $Label) {
    Assert-WindowsAbsolutePath $Root "$Label runtime root"
    if (-not (Test-Path -LiteralPath $Root -PathType Container)) {
        throw "$Label runtime directory is missing"
    }
    $RootItem = Get-Item -LiteralPath $Root -Force -ErrorAction Stop
    Assert-NoWindowsReparseAncestor $RootItem.FullName "$Label runtime root"

    $Separators = [char[]]@(
        [IO.Path]::DirectorySeparatorChar,
        [IO.Path]::AltDirectorySeparatorChar
    )
    $RootPath = [IO.Path]::GetFullPath($RootItem.FullName)
    $VolumeRoot = [IO.Path]::GetPathRoot($RootPath)
    if ($RootPath.Length -gt $VolumeRoot.Length) {
        $RootPath = $RootPath.TrimEnd($Separators)
    }
    $RootPrefix = $RootPath + [IO.Path]::DirectorySeparatorChar
    $Files = [Collections.Generic.SortedDictionary[string, object]]::new(
        [StringComparer]::Ordinal
    )
    $Directories = [Collections.Generic.SortedDictionary[string, object]]::new(
        [StringComparer]::Ordinal
    )
    $Pending = [Collections.Generic.Queue[IO.DirectoryInfo]]::new()
    $Pending.Enqueue($RootItem)
    $EntryCount = 0
    [long] $TotalBytes = 0

    while ($Pending.Count -gt 0) {
        $Current = $Pending.Dequeue()
        $Entries = @(Get-ChildItem -LiteralPath $Current.FullName -Force -ErrorAction Stop)
        foreach ($Entry in $Entries) {
            $EntryCount += 1
            if ($EntryCount -gt $script:WindowsRuntimeMaxEntries) {
                throw "$Label runtime tree contains too many entries"
            }
            if (($Entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "$Label runtime tree contains a reparse point"
            }
            $Relative = Get-WindowsRuntimeRelativePath $RootPrefix $Entry.FullName $Label
            if ($Entry.PSIsContainer) {
                if ($Directories.ContainsKey($Relative)) {
                    throw "$Label runtime tree contains a duplicate directory path"
                }
                $Directories.Add($Relative, $true)
                $Pending.Enqueue($Entry)
                continue
            }
            if ($Files.ContainsKey($Relative)) {
                throw "$Label runtime tree contains a duplicate file path"
            }
            [long] $Length = $Entry.Length
            $TotalBytes += $Length
            if ($TotalBytes -gt $script:WindowsRuntimeMaxBytes) {
                throw "$Label runtime tree exceeds the size limit"
            }
            $Hash = (Get-FileHash -LiteralPath $Entry.FullName -Algorithm SHA256).Hash
            $AfterHash = Get-Item -LiteralPath $Entry.FullName -Force -ErrorAction Stop
            if (($AfterHash.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or
                $AfterHash.Length -ne $Length -or
                $AfterHash.LastWriteTimeUtc -ne $Entry.LastWriteTimeUtc) {
                throw "$Label runtime file changed while it was being verified"
            }
            $Files.Add($Relative, [PSCustomObject]@{
                Length = $Length
                Sha256 = $Hash
            })
        }
    }
    if ($Files.Count -eq 0 -or -not $Files.ContainsKey("llama-server.exe")) {
        throw "$Label runtime tree does not contain llama-server.exe"
    }
    return [PSCustomObject]@{
        Files = $Files
        Directories = $Directories
        EntryCount = $EntryCount
        TotalBytes = $TotalBytes
    }
}

function Get-WindowsPackagedRuntimeRoot(
    [string] $PackagedDesktop,
    [string] $PackagedLlamaServer
) {
    $DesktopPath = Get-VerifiedWindowsRegularFile $PackagedDesktop "packaged desktop"
    $ServerPath = Get-VerifiedWindowsRegularFile `
        $PackagedLlamaServer `
        "packaged llama-server.exe"
    $PayloadRoot = [IO.Path]::GetDirectoryName($DesktopPath)
    $RuntimeRoot = Join-Path $PayloadRoot "llama"
    $ExpectedServer = Join-Path $RuntimeRoot "llama-server.exe"
    if (-not $ServerPath.Equals(
        [IO.Path]::GetFullPath($ExpectedServer),
        [StringComparison]::OrdinalIgnoreCase
    )) {
        throw "packaged llama.cpp runtime is not in the expected llama directory"
    }
    return $RuntimeRoot
}

function Assert-WindowsRuntimeTreeMatches(
    [string] $ExpectedRoot,
    [string] $ActualRoot
) {
    $Expected = Get-VerifiedRuntimeTree $ExpectedRoot "verified source"
    $Actual = Get-VerifiedRuntimeTree $ActualRoot "packaged"
    if ($Expected.Files.Count -ne $Actual.Files.Count -or
        $Expected.Directories.Count -ne $Actual.Directories.Count) {
        throw "packaged llama.cpp runtime file set differs from the verified source"
    }
    foreach ($Relative in $Expected.Directories.Keys) {
        if (-not $Actual.Directories.ContainsKey($Relative)) {
            throw "packaged llama.cpp runtime directory set differs from the verified source"
        }
    }
    foreach ($Relative in $Expected.Files.Keys) {
        if (-not $Actual.Files.ContainsKey($Relative)) {
            throw "packaged llama.cpp runtime file set differs from the verified source"
        }
        if ($Expected.Files[$Relative].Length -ne $Actual.Files[$Relative].Length -or
            $Expected.Files[$Relative].Sha256 -ne $Actual.Files[$Relative].Sha256) {
            throw "packaged llama.cpp runtime bytes differ from the verified source"
        }
    }
}

function Get-WindowsLlamaRuntimeManifest(
    [string] $RuntimeRoot,
    [string] $PolicyPath
) {
    $Tree = Get-VerifiedRuntimeTree $RuntimeRoot "verified llama.cpp"
    if ($Tree.Directories.Count -ne 0 -or $Tree.Files.Count -ne 2 -or
        -not $Tree.Files.ContainsKey("llama-server.exe") -or
        -not $Tree.Files.ContainsKey("BUILD-MANIFEST.json")) {
        throw "verified llama.cpp runtime must contain only llama-server.exe and BUILD-MANIFEST.json"
    }
    if ($Tree.Files["BUILD-MANIFEST.json"].Length -gt 65536) {
        throw "verified llama.cpp build manifest is too large"
    }

    $ResolvedPolicy = Get-VerifiedWindowsRegularFile `
        $PolicyPath `
        "llama.cpp Windows build policy"
    $ManifestPath = Get-VerifiedWindowsRegularFile `
        (Join-Path $RuntimeRoot "BUILD-MANIFEST.json") `
        "llama.cpp Windows build manifest"
    try {
        $Policy = Get-Content -LiteralPath $ResolvedPolicy -Raw | ConvertFrom-Json
        $Manifest = Get-Content -LiteralPath $ManifestPath -Raw | ConvertFrom-Json
    } catch {
        throw "llama.cpp Windows build policy or manifest is invalid JSON"
    }

    $PolicyHash = (Get-FileHash -LiteralPath $ResolvedPolicy -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($Manifest.schema_version -ne 1 -or $Manifest.component -ne "llama.cpp" -or
        $Manifest.tag -ne $Policy.tag -or $Manifest.commit -ne $Policy.commit -or
        $Manifest.policy_sha256 -ne $PolicyHash -or
        $Manifest.source.url -ne $Policy.source.url -or
        [long] $Manifest.source.size -ne [long] $Policy.source.size -or
        $Manifest.source.sha256 -ne $Policy.source.sha256 -or
        $Manifest.target.triple -ne $Policy.target.triple -or
        $Manifest.target.minimum_cpu -ne $Policy.target.minimum_cpu) {
        throw "llama.cpp Windows build manifest does not match the reviewed source policy"
    }
    $ExpectedCompilerFlags = @($Policy.build.compiler_flags) + @(
        "/pathmap:<SOURCE>=llama.cpp",
        "/pathmap:<BUILD>=build"
    )
    if ($Manifest.build.openmp -ne $false -or
        $Manifest.build.shared_libraries -ne $false -or
        $Manifest.build.msvc_runtime -ne "MultiThreaded" -or
        [long] $Manifest.build.source_date_epoch -ne [long] $Policy.source_date_epoch -or
        -not (Test-WindowsOrdinalSequenceEqual `
            $ExpectedCompilerFlags `
            @($Manifest.build.compiler_flags)) -or
        -not (Test-WindowsOrdinalSequenceEqual `
            @($Policy.build.cmake_definitions) `
            @($Manifest.build.cmake_definitions)) -or
        -not (Test-WindowsOrdinalSequenceEqual `
            @($Policy.build.linker_flags) `
            @($Manifest.build.linker_flags))) {
        throw "llama.cpp Windows build manifest weakened a required build invariant"
    }
    $PolicyPatches = ($Policy.source_patches | ConvertTo-Json -Compress -Depth 5)
    $ManifestPatches = ($Manifest.build.source_patches | ConvertTo-Json -Compress -Depth 5)
    if ($ManifestPatches -cne $PolicyPatches) {
        throw "llama.cpp Windows build manifest lacks the reviewed source patch receipt"
    }
    if (-not $Manifest.toolchain.visual_studio_installation_version.StartsWith(
            $Policy.toolchain.visual_studio_installation_version_prefix,
            [StringComparison]::Ordinal
        ) -or
        -not $Manifest.toolchain.vc_tools_version.StartsWith(
            $Policy.toolchain.vc_tools_version_prefix,
            [StringComparison]::Ordinal
        ) -or
        $Manifest.toolchain.windows_sdk_version -ne $Policy.toolchain.windows_sdk_version -or
        $Manifest.toolchain.cmake_version -ne $Policy.toolchain.cmake_version -or
        $Policy.toolchain.ninja_versions -notcontains $Manifest.toolchain.ninja_version) {
        throw "llama.cpp Windows build manifest used an unreviewed toolchain"
    }
    $ToolEntries = @($Manifest.toolchain.files.PSObject.Properties)
    if ($ToolEntries.Count -ne 11 -or
        [string]::IsNullOrWhiteSpace([string] $Manifest.toolchain.runner_image_version)) {
        throw "llama.cpp Windows build manifest has an incomplete toolchain receipt"
    }
    foreach ($ToolName in @(
        "cmd", "curl", "tar", "cmake", "ninja", "cl", "link", "lib", "rc", "mt", "dumpbin"
    )) {
        $ToolProperty = $Manifest.toolchain.files.PSObject.Properties[$ToolName]
        if ($null -eq $ToolProperty -or
            [string]::IsNullOrWhiteSpace([string] $ToolProperty.Value.file_version) -or
            [long] $ToolProperty.Value.size -le 0 -or
            $ToolProperty.Value.sha256 -notmatch '^[0-9a-f]{64}$') {
            throw "llama.cpp Windows build manifest has an incomplete toolchain receipt"
        }
    }
    if ($Manifest.toolchain.files.ninja.file_version -ne $Manifest.toolchain.ninja_version) {
        throw "llama.cpp Windows Ninja tool receipt does not match the reviewed toolchain"
    }
    $ExpectedToolBindings = [ordered]@{
        c_compiler = "cl"
        cxx_compiler = "cl"
        asm_compiler = "cl"
        linker = "link"
        librarian = "lib"
        resource_compiler = "rc"
        manifest_tool = "mt"
        build_program = "ninja"
    }
    $ExpectedToolBindingsJson = $ExpectedToolBindings | ConvertTo-Json -Compress
    $ManifestToolBindingsJson = $Manifest.build.tool_bindings | ConvertTo-Json -Compress
    if ($ManifestToolBindingsJson -cne $ExpectedToolBindingsJson) {
        throw "llama.cpp Windows build manifest has unreviewed native tool bindings"
    }

    $RuntimeFiles = @($Manifest.runtime.files)
    if ($RuntimeFiles.Count -ne 1 -or $RuntimeFiles[0].path -ne "llama-server.exe" -or
        [long] $RuntimeFiles[0].size -ne [long] $Tree.Files["llama-server.exe"].Length -or
        $RuntimeFiles[0].sha256 -ne $Tree.Files["llama-server.exe"].Sha256.ToLowerInvariant()) {
        throw "llama.cpp Windows build manifest does not authenticate llama-server.exe"
    }
    $ReproducibilityProperty = $Manifest.PSObject.Properties["reproducibility"]
    if ($null -eq $ReproducibilityProperty) {
        throw "llama.cpp Windows build manifest lacks the required two-build reproducibility receipt"
    }
    $Reproducibility = $ReproducibilityProperty.Value
    $ReproducibilityBuilds = @($Reproducibility.builds)
    if ([int] $Reproducibility.build_count -ne 2 -or
        $Reproducibility.isolated_work_roots -ne $true -or
        $Reproducibility.matching_outputs -ne $true -or
        $ReproducibilityBuilds.Count -ne 2) {
        throw "llama.cpp Windows build manifest lacks the required two-build reproducibility receipt"
    }
    for ($Index = 0; $Index -lt 2; $Index += 1) {
        $BuildReceipt = $ReproducibilityBuilds[$Index]
        if ([int] $BuildReceipt.ordinal -ne ($Index + 1) -or
            [long] $BuildReceipt.size -ne [long] $RuntimeFiles[0].size -or
            $BuildReceipt.sha256 -ne $RuntimeFiles[0].sha256 -or
            $BuildReceipt.sha256 -notmatch '^[0-9a-f]{64}$') {
            throw "llama.cpp Windows reproducibility receipt does not authenticate both builds"
        }
    }
    if ($RuntimeFiles[0].sha256 -notmatch '^[0-9a-f]{64}$' -or
        $Manifest.runtime.version_output -notmatch "version: 9946 \($([regex]::Escape($Policy.commit))\)" -or
        @(Compare-Object @($Policy.target.imports | Sort-Object -Unique) `
            @($Manifest.runtime.imports | Sort-Object -Unique)).Count -ne 0 -or
        @($Manifest.runtime.imports | Where-Object { $_ -match '(?i)(omp|vcruntime|msvcp|ucrt)' }).Count -ne 0) {
        throw "llama.cpp Windows runtime dependencies or smoke receipt are outside policy"
    }
    return $Manifest
}

function Assert-WindowsDesktopEmbedsLlamaRuntimeHash(
    [string] $DesktopPath,
    [string] $RuntimeRoot,
    [string] $PolicyPath
) {
    $Desktop = Get-VerifiedWindowsRegularFile $DesktopPath "Windows desktop executable"
    if ((Get-Item -LiteralPath $Desktop).Length -gt 512MB) {
        throw "Windows desktop executable is too large for runtime receipt verification"
    }
    $Manifest = Get-WindowsLlamaRuntimeManifest $RuntimeRoot $PolicyPath
    $ExpectedHash = [string] @($Manifest.runtime.files)[0].sha256
    $DesktopText = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($Desktop))
    if (-not $DesktopText.Contains($ExpectedHash)) {
        throw "Windows desktop executable does not embed this llama.cpp runtime hash"
    }
}
