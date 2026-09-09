use std::{ffi::OsString, io::Read, path::PathBuf};

use serde::Deserialize;

use super::{CommandRunner, CommandSpec, IntegrationEnvironment, MAX_PROCESS_OUTPUT, Path, Result};

/// The executable and fixed arguments are kept together so npm entry points
/// run through Node directly, never through cmd.exe or a Unix npm shim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ClientCommand {
    executable: PathBuf,
    prefix_args: Vec<OsString>,
}

impl ClientCommand {
    pub(super) fn native(executable: PathBuf) -> Self {
        Self {
            executable,
            prefix_args: Vec::new(),
        }
    }

    pub(super) fn command(&self) -> CommandSpec {
        CommandSpec::new(self.executable.clone()).args(self.prefix_args.iter().cloned())
    }
}

type Detection<T> = std::result::Result<Option<T>, &'static str>;

#[derive(Debug, Clone)]
pub(super) struct WindowsClients {
    pub(super) codex: Detection<ClientCommand>,
    pub(super) codex_config: Option<PathBuf>,
    pub(super) claude_code: Detection<ClientCommand>,
    pub(super) gemini: Detection<ClientCommand>,
    pub(super) claude_desktop: Detection<PathBuf>,
}

#[derive(Debug)]
struct WindowsPaths {
    home: PathBuf,
    codex_home: Option<PathBuf>,
    search: Vec<PathBuf>,
    local_app_data: PathBuf,
    app_data: PathBuf,
    program_files: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
struct WindowsInventory {
    path_entries: Vec<PathBuf>,
    packages: Vec<InstalledPackage>,
}

#[derive(Debug, Deserialize)]
struct InstalledPackage {
    name: String,
    location: PathBuf,
    executables: Vec<String>,
}

pub(super) async fn refresh_windows(
    environment: &IntegrationEnvironment,
    runner: &dyn CommandRunner,
) -> Result<WindowsClients> {
    let home = environment.home.clone();
    let paths = WindowsPaths {
        codex_home: std::env::var_os("CODEX_HOME").map(PathBuf::from),
        local_app_data: std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Local")),
        app_data: std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Roaming")),
        program_files: std::env::var_os("ProgramFiles").map(PathBuf::from),
        search: std::env::var_os("PATH")
            .map(|value| std::env::split_paths(&value).collect())
            .unwrap_or_else(|| environment.path_entries.clone()),
        home,
    };
    let inventory = match std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .filter(|root| local_absolute(root))
    {
        Some(root) => runner
            .run(
                CommandSpec::new(root.join("System32/WindowsPowerShell/v1.0/powershell.exe")).args(
                    [
                        "-NoLogo",
                        "-NoProfile",
                        "-NonInteractive",
                        "-Command",
                        include_str!("windows-discovery.ps1"),
                    ],
                ),
            )
            .await
            .ok()
            .filter(|output| output.success)
            .and_then(|output| serde_json::from_slice::<WindowsInventory>(&output.stdout).ok()),
        None => None,
    };
    // Registry/package metadata and executable probes never traverse files on
    // the async worker. A failed inventory does not hide independently found CLIs.
    tokio::task::spawn_blocking(move || resolve_windows(paths, inventory))
        .await
        .map_err(|_| anyhow::anyhow!("no se pudo completar la detección de aplicaciones"))
}

pub(super) fn local_absolute(path: &Path) -> bool {
    if !path.is_absolute() {
        return false;
    }
    #[cfg(windows)]
    {
        matches!(path.components().next(), Some(std::path::Component::Prefix(prefix))
            if matches!(prefix.kind(), std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)))
    }
    #[cfg(not(windows))]
    {
        true
    }
}

fn local_file(path: &Path) -> bool {
    // Native installers may use local junctions. Resolve them without applying
    // the stricter managed-capability rules, but never select a share target.
    std::fs::canonicalize(path)
        .ok()
        .is_some_and(|target| local_absolute(&target) && target.is_file())
}

fn resolve_windows(mut paths: WindowsPaths, inventory: Option<WindowsInventory>) -> WindowsClients {
    let inventory_failed = inventory.is_none();
    let inventory = inventory.unwrap_or_default();
    // Persisted PATH catches tools installed after Explorer/AirWiki started.
    paths.search.extend(inventory.path_entries);
    paths.search.extend([
        paths.home.join(".local/bin"),
        paths.app_data.join("npm"),
        paths.local_app_data.join("Programs/OpenAI/Codex/bin"),
    ]);
    if let Some(program_files) = &paths.program_files {
        paths.search.push(program_files.join("nodejs"));
    }
    paths.search.retain(|path| local_absolute(path));
    let codex = find_windows_cli("codex", "@openai/codex", &paths.search);
    let claude_code = find_windows_cli("claude", "@anthropic-ai/claude-code", &paths.search);
    let gemini = find_windows_cli("gemini", "@google/gemini-cli", &paths.search);
    let (codex, codex_config) = if matches!(codex, Ok(None)) {
        match find_packaged_codex_config(&paths, &inventory.packages, inventory_failed) {
            Ok(config) => (codex, config),
            Err(error) => (Err(error), None),
        }
    } else {
        (codex, None)
    };
    let claude_desktop = find_windows_claude(&paths, &inventory.packages, inventory_failed);
    WindowsClients {
        codex,
        codex_config,
        claude_code,
        gemini,
        claude_desktop,
    }
}

fn find_windows_cli(name: &str, package: &str, search: &[PathBuf]) -> Detection<ClientCommand> {
    for directory in search {
        let executable = directory.join(format!("{name}.exe"));
        if local_file(&executable) {
            return Ok(Some(ClientCommand::native(executable)));
        }
        // A Windows npm install has .cmd/.ps1 launchers beside its extensionless
        // Unix shim. Presence alone must never lead us to execute that Unix file.
        if directory.join(format!("{name}.cmd")).is_file() {
            let root = directory.join("node_modules").join(package);
            let script = npm_entry_point(&root, name, package).ok_or(
                "La instalación de la herramienta está incompleta o su lanzador no es compatible.",
            )?;
            let node = std::iter::once(directory)
                .chain(search.iter())
                .map(|path| path.join("node.exe"))
                .find(|path| local_file(path))
                .ok_or(
                    "La herramienta está instalada, pero no se encontró Node.js para ejecutarla.",
                )?;
            return Ok(Some(ClientCommand {
                executable: node,
                prefix_args: vec![script.into_os_string()],
            }));
        }
    }
    Ok(None)
}

fn safe_relative(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with(['/', '\\'])
        && !value.contains(':')
        && !value.split(['/', '\\']).any(|part| part == "..")
}

fn npm_entry_point(root: &Path, name: &str, package: &str) -> Option<PathBuf> {
    let root = std::fs::canonicalize(root).ok()?;
    if !local_absolute(&root) {
        return None;
    }
    let manifest = std::fs::canonicalize(root.join("package.json")).ok()?;
    if !manifest.starts_with(&root) {
        return None;
    }
    let file = std::fs::File::open(manifest).ok()?;
    let mut bytes = Vec::new();
    file.take(MAX_PROCESS_OUTPUT as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > MAX_PROCESS_OUTPUT {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    if value.get("name")?.as_str()? != package {
        return None;
    }
    let bin = value.get("bin")?;
    let relative = bin.as_str().or_else(|| bin.get(name)?.as_str())?;
    if !safe_relative(relative) {
        return None;
    }
    let entry = std::fs::canonicalize(root.join(relative)).ok()?;
    (entry.starts_with(&root) && entry.is_file()).then_some(entry)
}

fn find_packaged_codex_config(
    paths: &WindowsPaths,
    packages: &[InstalledPackage],
    failed: bool,
) -> Detection<PathBuf> {
    // MSIX registration proves installation, not permission to execute files
    // inside WindowsApps. Use Codex's documented user configuration instead.
    if packages.iter().any(|package| {
        matches!(
            package.name.as_str(),
            "OpenAI.Codex" | "OpenAI.ChatGPT-Desktop"
        ) && local_absolute(&package.location)
    }) {
        let root = paths
            .codex_home
            .clone()
            .unwrap_or_else(|| paths.home.join(".codex"));
        if !local_absolute(&root) {
            return Err("La configuración de Codex debe usar una ruta local absoluta.");
        }
        return Ok(Some(root.join("config.toml")));
    }
    if packages.iter().any(|package| {
        matches!(
            package.name.as_str(),
            "OpenAI.Codex" | "OpenAI.ChatGPT-Desktop"
        )
    }) {
        return Err(
            "Windows devolvió una ubicación no compatible para Codex; actualiza el estado.",
        );
    }
    if failed {
        return Err(
            "Windows no pudo consultar las aplicaciones registradas. Actualiza el estado para reintentar.",
        );
    }
    Ok(None)
}

fn find_windows_claude(
    paths: &WindowsPaths,
    packages: &[InstalledPackage],
    failed: bool,
) -> Detection<PathBuf> {
    for package in packages
        .iter()
        .filter(|package| package.name == "Claude" && local_absolute(&package.location))
    {
        for relative in package
            .executables
            .iter()
            .filter(|value| safe_relative(value))
        {
            let executable = package.location.join(relative);
            if executable
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("Claude.exe"))
                && local_file(&executable)
            {
                return Ok(Some(executable));
            }
        }
    }
    for relative in [
        "Programs/Claude/Claude.exe",
        "AnthropicClaude/Claude.exe",
        "Claude/Claude.exe",
    ] {
        let executable = paths.local_app_data.join(relative);
        if local_absolute(&executable) && local_file(&executable) {
            return Ok(Some(executable));
        }
    }
    // Older Squirrel installations keep Claude.exe inside app-<version>.
    let root = paths.local_app_data.join("AnthropicClaude");
    if local_absolute(&root)
        && let Ok(entries) = std::fs::read_dir(root)
    {
        let newest = entries
            .take(128)
            .filter_map(std::result::Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name();
                let version: Vec<u32> = name
                    .to_str()?
                    .strip_prefix("app-")?
                    .split('.')
                    .map(str::parse)
                    .collect::<std::result::Result<_, _>>()
                    .ok()?;
                let executable = entry.path().join("Claude.exe");
                local_file(&executable).then_some((version, executable))
            })
            .max_by(|left, right| left.0.cmp(&right.0));
        if let Some((_, executable)) = newest {
            return Ok(Some(executable));
        }
    }
    if packages.iter().any(|package| package.name == "Claude") {
        return Err(
            "Claude Desktop está instalado, pero no se encontró su ejecutable. Repara la instalación y actualiza el estado.",
        );
    }
    if failed {
        return Err(
            "Windows no pudo consultar las aplicaciones registradas. Actualiza el estado para reintentar.",
        );
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::super::{ChatClientKind, CommandOutput, HostPlatform, IntegrationStatus};
    use super::*;
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    fn fixture_paths(temp: &TempDir) -> WindowsPaths {
        let root = std::fs::canonicalize(temp.path()).unwrap();
        WindowsPaths {
            codex_home: None,
            home: root.join("User with spaces & accents é"),
            local_app_data: root.join("redirected-local"),
            app_data: root.join("redirected-roaming"),
            program_files: Some(root.join("Program Files")),
            search: Vec::new(),
        }
    }

    fn file(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"synthetic fixture").unwrap();
    }

    fn npm_install(prefix: &Path, package: &str, name: &str) -> PathBuf {
        file(&prefix.join(name));
        file(&prefix.join(format!("{name}.cmd")));
        let root = prefix.join("node_modules").join(package);
        let entry = root.join("bin/cli.js");
        file(&entry);
        std::fs::write(
            root.join("package.json"),
            serde_json::to_vec(&serde_json::json!({
                "name": package, "bin": {name: "./bin/cli.js"}
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::canonicalize(entry).unwrap()
    }

    #[test]
    fn discovers_native_cli_installers_with_an_empty_process_path() {
        let temp = TempDir::new().unwrap();
        let paths = fixture_paths(&temp);
        let codex = paths
            .local_app_data
            .join("Programs/OpenAI/Codex/bin/codex.exe");
        let claude = paths.home.join(".local/bin/claude.exe");
        file(&codex);
        file(&claude);
        let found = resolve_windows(paths, Some(WindowsInventory::default()));
        assert_eq!(found.codex.unwrap(), Some(ClientCommand::native(codex)));
        assert_eq!(
            found.claude_code.unwrap(),
            Some(ClientCommand::native(claude))
        );
    }

    #[test]
    fn npm_tools_use_node_and_their_entry_points_instead_of_batch_or_unix_shims() {
        let temp = TempDir::new().unwrap();
        let paths = fixture_paths(&temp);
        let prefix = paths.app_data.join("npm");
        let node = paths
            .program_files
            .as_ref()
            .unwrap()
            .join("nodejs/node.exe");
        file(&node);
        let entries = [
            npm_install(&prefix, "@openai/codex", "codex"),
            npm_install(&prefix, "@anthropic-ai/claude-code", "claude"),
            npm_install(&prefix, "@google/gemini-cli", "gemini"),
        ];
        let found = resolve_windows(paths, Some(WindowsInventory::default()));
        for (detected, entry) in [found.codex, found.claude_code, found.gemini]
            .into_iter()
            .zip(entries)
        {
            let command = detected
                .unwrap()
                .unwrap()
                .command()
                .args(["mcp", "get", "airwiki"]);
            assert_eq!(command.executable, node);
            assert_eq!(
                command.args,
                [
                    entry.into_os_string(),
                    "mcp".into(),
                    "get".into(),
                    "airwiki".into()
                ]
            );
        }
    }

    #[test]
    fn persisted_path_finds_a_custom_install_added_after_airwiki_started() {
        let temp = TempDir::new().unwrap();
        let paths = fixture_paths(&temp);
        let directory = temp.path().join("custom installation");
        let executable = directory.join("claude.exe");
        file(&executable);
        let found = resolve_windows(
            paths,
            Some(WindowsInventory {
                path_entries: vec![directory],
                packages: Vec::new(),
            }),
        );
        assert_eq!(
            found.claude_code.unwrap(),
            Some(ClientCommand::native(executable))
        );
    }

    #[test]
    fn current_user_msix_packages_make_both_desktop_integrations_discoverable() {
        let temp = TempDir::new().unwrap();
        let paths = fixture_paths(&temp);
        let codex_root = temp.path().join("WindowsApps/OpenAI.Codex_1.0_x64");
        let claude_root = temp.path().join("WindowsApps/Claude_1.0_x64");
        let codex = codex_root.join("app/resources/codex.exe");
        let claude = claude_root.join("app/Claude.exe");
        let config = paths.home.join(".codex/config.toml");
        file(&codex);
        file(&claude);
        let found = resolve_windows(
            paths,
            Some(WindowsInventory {
                path_entries: Vec::new(),
                packages: vec![
                    InstalledPackage {
                        name: "OpenAI.Codex".into(),
                        location: codex_root,
                        executables: vec!["app/ChatGPT.exe".into()],
                    },
                    InstalledPackage {
                        name: "Claude".into(),
                        location: claude_root,
                        executables: vec!["app/Claude.exe".into()],
                    },
                ],
            }),
        );
        assert_eq!(found.codex.unwrap(), None);
        assert_eq!(found.codex_config, Some(config));
        assert_eq!(found.claude_desktop.unwrap(), Some(claude));
    }

    #[test]
    fn blocked_package_inventory_does_not_hide_a_locally_installed_cli() {
        let temp = TempDir::new().unwrap();
        let paths = fixture_paths(&temp);
        file(&paths.home.join(".local/bin/claude.exe"));
        let found = resolve_windows(paths, None);
        assert!(found.claude_code.unwrap().is_some());
        assert!(found.claude_desktop.is_err());
        assert!(found.codex.is_err());
    }

    #[test]
    fn a_missing_node_runtime_is_a_recoverable_error_not_an_absent_app() {
        let temp = TempDir::new().unwrap();
        let paths = fixture_paths(&temp);
        npm_install(&paths.app_data.join("npm"), "@google/gemini-cli", "gemini");
        let found = resolve_windows(paths, Some(WindowsInventory::default()));
        assert!(found.gemini.unwrap_err().contains("Node.js"));
    }

    #[test]
    fn invalid_npm_metadata_cannot_launch_a_file_outside_the_package() {
        let temp = TempDir::new().unwrap();
        let prefix = temp.path().join("npm");
        npm_install(&prefix, "@google/gemini-cli", "gemini");
        let root = prefix.join("node_modules/@google/gemini-cli");
        for relative in [
            "../../../outside.js",
            r"..\outside.js",
            r"C:\outside.js",
            "/outside.js",
        ] {
            std::fs::write(
                root.join("package.json"),
                serde_json::to_vec(&serde_json::json!({
                    "name": "@google/gemini-cli", "bin": {"gemini": relative}
                }))
                .unwrap(),
            )
            .unwrap();
            assert!(npm_entry_point(&root, "gemini", "@google/gemini-cli").is_none());
        }
    }

    #[test]
    fn an_extensionless_unix_shim_is_not_a_windows_executable() {
        let temp = TempDir::new().unwrap();
        let mut paths = fixture_paths(&temp);
        paths.search.push(temp.path().to_path_buf());
        file(&temp.path().join("gemini"));
        let found = resolve_windows(paths, Some(WindowsInventory::default()));
        assert_eq!(found.gemini.unwrap(), None);
    }

    #[test]
    fn malformed_or_oversized_npm_manifests_are_rejected() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        for bytes in [b"not json".to_vec(), vec![b' '; MAX_PROCESS_OUTPUT + 1]] {
            std::fs::write(root.join("package.json"), bytes).unwrap();
            assert!(npm_entry_point(root, "gemini", "@google/gemini-cli").is_none());
        }
    }

    #[cfg(unix)]
    #[test]
    fn npm_symlinks_cannot_escape_the_package_boundary() {
        let temp = TempDir::new().unwrap();
        let prefix = temp.path().join("npm");
        let entry = npm_install(&prefix, "@google/gemini-cli", "gemini");
        let root = prefix.join("node_modules/@google/gemini-cli");
        let outside = temp.path().join("outside.js");
        file(&outside);
        std::fs::remove_file(&entry).unwrap();
        std::os::unix::fs::symlink(&outside, &entry).unwrap();
        assert!(npm_entry_point(&root, "gemini", "@google/gemini-cli").is_none());
        std::fs::remove_file(&entry).unwrap();
        file(&entry);

        let manifest = root.join("package.json");
        let external_manifest = temp.path().join("external-package.json");
        std::fs::rename(&manifest, &external_manifest).unwrap();
        std::os::unix::fs::symlink(&external_manifest, &manifest).unwrap();
        assert!(npm_entry_point(&root, "gemini", "@google/gemini-cli").is_none());
    }

    #[test]
    fn a_registered_claude_package_with_an_invalid_manifest_is_not_absent() {
        let temp = TempDir::new().unwrap();
        let found = resolve_windows(
            fixture_paths(&temp),
            Some(WindowsInventory {
                path_entries: Vec::new(),
                packages: vec![InstalledPackage {
                    name: "Claude".into(),
                    location: temp.path().join("WindowsApps/Claude"),
                    executables: vec!["../Claude.exe".into()],
                }],
            }),
        );
        assert!(found.claude_desktop.unwrap_err().contains("Repara"));
    }

    #[test]
    fn an_installed_chatgpt_package_without_a_cli_is_not_reported_as_absent() {
        let temp = TempDir::new().unwrap();
        let found = resolve_windows(
            fixture_paths(&temp),
            Some(WindowsInventory {
                path_entries: Vec::new(),
                packages: vec![InstalledPackage {
                    name: "OpenAI.ChatGPT-Desktop".into(),
                    location: temp.path().to_path_buf(),
                    executables: Vec::new(),
                }],
            }),
        );
        assert_eq!(found.codex.unwrap(), None);
        assert_eq!(
            found.codex_config,
            Some(fixture_paths(&temp).home.join(".codex/config.toml"))
        );
    }

    #[test]
    fn msix_configuration_respects_codex_home_and_rejects_a_relative_override() {
        let temp = TempDir::new().unwrap();
        for root in [
            temp.path().join("custom Codex ñ"),
            PathBuf::from("relative"),
        ] {
            let mut paths = fixture_paths(&temp);
            paths.codex_home = Some(root.clone());
            let found = resolve_windows(
                paths,
                Some(WindowsInventory {
                    path_entries: Vec::new(),
                    packages: vec![InstalledPackage {
                        name: "OpenAI.Codex".into(),
                        location: temp.path().to_path_buf(),
                        executables: Vec::new(),
                    }],
                }),
            );
            if root.is_absolute() {
                assert_eq!(found.codex_config, Some(root.join("config.toml")));
                assert_eq!(found.codex.unwrap(), None);
            } else {
                assert!(found.codex.is_err());
                assert!(found.codex_config.is_none());
            }
        }
    }

    #[test]
    fn legacy_claude_installation_selects_the_newest_numeric_version() {
        let temp = TempDir::new().unwrap();
        let paths = fixture_paths(&temp);
        let root = paths.local_app_data.join("AnthropicClaude");
        file(&root.join("app-1.9.0/Claude.exe"));
        let newest = root.join("app-1.10.0/Claude.exe");
        file(&newest);
        let found = resolve_windows(paths, Some(WindowsInventory::default()));
        assert_eq!(found.claude_desktop.unwrap(), Some(newest));
    }

    #[derive(Default)]
    struct CliRunner(Mutex<Vec<CommandSpec>>);

    #[async_trait]
    impl CommandRunner for CliRunner {
        async fn run(&self, spec: CommandSpec) -> Result<CommandOutput> {
            let is_get = spec.args.iter().any(|arg| arg == "get")
                && !spec.args.iter().any(|arg| arg == "--help");
            self.0.lock().unwrap().push(spec);
            Ok(CommandOutput {
                success: !is_get,
                stdout: b"--json --scope --transport --include-tools".to_vec(),
                _stderr: if is_get {
                    b"No MCP server named 'airwiki' found.".to_vec()
                } else {
                    Vec::new()
                },
            })
        }
    }

    #[tokio::test]
    async fn msix_detection_unlocks_connect_without_granting_ai_access() {
        let temp = TempDir::new().unwrap();
        let codex_root = temp.path().join("WindowsApps/OpenAI.Codex_1.0_x64");
        let claude_root = temp.path().join("WindowsApps/Claude_1.0_x64");
        file(&codex_root.join("app/resources/codex.exe"));
        file(&claude_root.join("app/Claude.exe"));
        file(&temp.path().join("integrations/airwiki-claude.mcpb"));
        let clients = resolve_windows(
            fixture_paths(&temp),
            Some(WindowsInventory {
                path_entries: Vec::new(),
                packages: vec![
                    InstalledPackage {
                        name: "OpenAI.Codex".into(),
                        location: codex_root,
                        executables: Vec::new(),
                    },
                    InstalledPackage {
                        name: "Claude".into(),
                        location: claude_root,
                        executables: vec!["app/Claude.exe".into()],
                    },
                ],
            }),
        );
        let mut manager = super::super::tests::test_manager(&temp, temp.path().join("airwiki.exe"));
        manager.environment.platform = HostPlatform::Windows;
        manager.environment.discover_host_clients = true;
        manager.environment.windows_clients = Some(clients);
        let runner = Arc::new(CliRunner::default());
        manager.runner = runner.clone();
        assert_eq!(
            manager.inspect_chatgpt().await.unwrap().status,
            IntegrationStatus::Available
        );
        assert_eq!(
            manager.inspect_claude().await.unwrap().status,
            IntegrationStatus::Available
        );
        assert!(
            runner.0.lock().unwrap().is_empty(),
            "MSIX discovery must not spawn protected binaries"
        );
        assert!(
            !manager
                .capability_path(ChatClientKind::ChatGptDesktop)
                .exists()
        );
        assert!(
            !manager
                .capability_path(ChatClientKind::ClaudeDesktop)
                .exists()
        );
    }

    #[tokio::test]
    async fn npm_configuration_keeps_mcp_arguments_literal_and_uses_the_resolved_runtime() {
        let temp = TempDir::new().unwrap();
        let paths = fixture_paths(&temp);
        let prefix = paths.app_data.join("npm");
        let entry = npm_install(&prefix, "@openai/codex", "codex");
        let node = prefix.join("node.exe");
        file(&node);
        let codex = resolve_windows(paths, Some(WindowsInventory::default()))
            .codex
            .unwrap()
            .unwrap();
        let mut manager = super::super::tests::test_manager(&temp, temp.path().join("airwiki.exe"));
        let runner = Arc::new(CliRunner::default());
        manager.runner = runner.clone();
        let bridge = temp
            .path()
            .join("literal %VARIABLE% & space/airwiki-mcp-bridge.exe");
        manager
            .codex_add_configuration(
                &codex,
                &super::super::ManagedConfiguration::new(
                    bridge.clone(),
                    ChatClientKind::ChatGptDesktop,
                ),
            )
            .await
            .unwrap();
        let commands = runner.0.lock().unwrap();
        let [spec] = commands.as_slice() else {
            panic!("expected one configuration command");
        };
        assert_eq!(spec.executable, node);
        assert_eq!(
            spec.args,
            [
                entry.into_os_string(),
                "mcp".into(),
                "add".into(),
                "airwiki".into(),
                "--".into(),
                bridge.into_os_string(),
                "--client".into(),
                "chatgpt-desktop".into()
            ]
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn powershell_inventory_reads_mocked_current_user_packages_as_utf8_arrays() {
        // Exercise Windows PowerShell's real XML/JSON and encoding behavior;
        // the Appx commands are synthetic and never inspect installed clients.
        let temp = TempDir::new().unwrap();
        let root = std::fs::canonicalize(temp.path())
            .unwrap()
            .join("paquetes-é");
        let script = format!(
            r#"
function Get-AppxPackage {{
    [CmdletBinding()] param([string]$Name)
    if ($Name -eq 'Claude') {{
        [PSCustomObject]@{{ Name = 'Claude'; PackageFullName = 'fixture'; InstallLocation = $env:AIRWIKI_TEST_PACKAGE_ROOT }}
    }}
}}
function Get-AppxPackageManifest {{
    [CmdletBinding()] param([string]$Package)
    [xml]'<Package><Applications><Application Executable="app/Claude.exe" /></Applications></Package>'
}}
{}
"#,
            include_str!("windows-discovery.ps1")
        );
        let powershell = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let output = super::super::SystemCommandRunner
            .run(
                CommandSpec::new(powershell)
                    .args([
                        "-NoLogo",
                        "-NoProfile",
                        "-NonInteractive",
                        "-Command",
                        &script,
                    ])
                    .environment("AIRWIKI_TEST_PACKAGE_ROOT", root.as_os_str()),
            )
            .await
            .unwrap();
        assert!(output.success);
        let inventory: WindowsInventory = serde_json::from_slice(&output.stdout).unwrap();
        let [package] = inventory.packages.as_slice() else {
            panic!("expected one registered package");
        };
        assert_eq!(package.location, root);
        assert_eq!(package.executables, ["app/Claude.exe"]);
    }
}
