//! Codex MSIX uses the documented user TOML configuration without executing
//! protected package binaries. Only the managed MCP entry may be changed.

use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use toml_edit::{DocumentMut, Item, Table, value};
use uuid::Uuid;

use super::{INTEGRATION_NAME, ManagedConfiguration, discovery::local_absolute};
use crate::workflow_guides::ensure_path_has_no_links;

const MAX_CONFIG_BYTES: usize = 1024 * 1024;

pub(super) async fn read_configuration(path: &Path) -> Result<Option<ManagedConfiguration>> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || Snapshot::read(&path)?.configuration())
        .await
        .context("no se pudo completar la lectura de la configuración de Codex")?
}

pub(super) async fn replace_configuration(
    path: &Path,
    expected: Option<&ManagedConfiguration>,
    desired: Option<&ManagedConfiguration>,
) -> Result<()> {
    let path = path.to_path_buf();
    let expected = expected.cloned();
    let desired = desired.cloned();
    tokio::task::spawn_blocking(move || {
        let mut snapshot = Snapshot::read(&path)?;
        ensure!(
            snapshot.configuration()? == expected,
            "la configuración de Codex cambió; actualiza el estado antes de reintentar"
        );
        if expected == desired {
            return Ok(());
        }
        snapshot.set_configuration(desired.as_ref())?;
        let mut bytes = snapshot.document.to_string().into_bytes();
        if snapshot
            .original
            .as_deref()
            .is_some_and(|bytes| bytes.starts_with(&[0xef, 0xbb, 0xbf]))
        {
            bytes.splice(..0, [0xef, 0xbb, 0xbf]);
        }
        ensure!(
            bytes.len() <= MAX_CONFIG_BYTES,
            "la configuración de Codex es demasiado grande"
        );
        commit(&path, snapshot.original.as_deref(), &bytes)?;
        ensure!(
            Snapshot::read(&path)?.configuration()? == desired,
            "Codex cambió su configuración después de guardarla; actualiza el estado"
        );
        Ok(())
    })
    .await
    .context("no se pudo completar la configuración de Codex")?
}

struct Snapshot {
    original: Option<Vec<u8>>,
    document: DocumentMut,
}

impl Snapshot {
    fn read(path: &Path) -> Result<Self> {
        let original = read_bytes(path)?;
        let bytes = original.as_deref().unwrap_or_default();
        let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
        let text = std::str::from_utf8(bytes).context("la configuración de Codex no es UTF-8")?;
        // Parser diagnostics can contain user settings and secrets. Do not
        // propagate their source text to UI errors or logs.
        let document = text.parse().map_err(|_| {
            anyhow::anyhow!("La configuración de Codex contiene TOML inválido; no se modificó.")
        })?;
        Ok(Self { original, document })
    }

    fn configuration(&self) -> Result<Option<ManagedConfiguration>> {
        let Some(servers) = self.document.get("mcp_servers") else {
            return Ok(None);
        };
        let servers = servers.as_table().context(
            "Codex debe usar una tabla mcp_servers estándar; no se modificó la configuración",
        )?;
        let Some(entry) = servers.get(INTEGRATION_NAME) else {
            return Ok(None);
        };
        Ok(Some(parse_entry(entry)))
    }

    fn set_configuration(&mut self, desired: Option<&ManagedConfiguration>) -> Result<()> {
        if let Some(desired) = desired {
            let servers = self
                .document
                .as_table_mut()
                .entry("mcp_servers")
                .or_insert_with(|| Item::Table(Table::new()))
                .as_table_mut()
                .context("mcp_servers no es una tabla estándar")?;
            let mut entry = Table::new();
            entry.insert(
                "command",
                value(desired.command.to_str().context(
                    "la ruta del puente no se puede representar en la configuración de Codex",
                )?),
            );
            let args: toml_edit::Array = desired.args.iter().map(String::as_str).collect();
            entry.insert("args", value(args));
            servers.insert(INTEGRATION_NAME, Item::Table(entry));
        } else if let Some(servers) = self.document.get_mut("mcp_servers") {
            let servers = servers
                .as_table_mut()
                .context("mcp_servers no es una tabla estándar")?;
            servers.remove(INTEGRATION_NAME);
            if servers.is_empty() {
                self.document.as_table_mut().remove("mcp_servers");
            }
        }
        Ok(())
    }
}

fn parse_entry(entry: &Item) -> ManagedConfiguration {
    let Some(table) = entry.as_table_like() else {
        return ManagedConfiguration::conflict();
    };
    if table
        .iter()
        .any(|(key, _)| !matches!(key, "command" | "args" | "enabled"))
        || table
            .get("enabled")
            .is_some_and(|item| item.as_bool() != Some(true))
    {
        return ManagedConfiguration::conflict();
    }
    let Some(command) = table.get("command").and_then(Item::as_str) else {
        return ManagedConfiguration::conflict();
    };
    let args = match table.get("args") {
        None => Vec::new(),
        Some(item) => {
            let Some(args) = item.as_array().and_then(|array| {
                array
                    .iter()
                    .map(|arg| arg.as_str().map(str::to_owned))
                    .collect::<Option<Vec<_>>>()
            }) else {
                return ManagedConfiguration::conflict();
            };
            args
        }
    };
    ManagedConfiguration {
        command: PathBuf::from(command),
        args,
        parse_conflict: false,
    }
}

fn validate_path(path: &Path) -> Result<()> {
    ensure!(
        local_absolute(path)
            && !path
                .components()
                .any(|part| matches!(part, Component::ParentDir)),
        "la configuración de Codex debe usar una ruta local absoluta sin retrocesos"
    );
    ensure_path_has_no_links(path)
}

fn read_bytes(path: &Path) -> Result<Option<Vec<u8>>> {
    validate_path(path)?;
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).context("no se pudo inspeccionar la configuración de Codex");
        }
    };
    ensure!(
        metadata.is_file(),
        "la configuración de Codex no es un archivo regular"
    );
    let mut bytes = Vec::new();
    fs::File::open(path)
        .context("no se pudo leer la configuración de Codex")?
        .take(MAX_CONFIG_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .context("no se pudo leer la configuración de Codex")?;
    ensure!(
        bytes.len() <= MAX_CONFIG_BYTES,
        "la configuración de Codex es demasiado grande"
    );
    Ok(Some(bytes))
}

fn commit(path: &Path, original: Option<&[u8]>, bytes: &[u8]) -> Result<()> {
    validate_path(path)?;
    let parent = path
        .parent()
        .context("la configuración de Codex no tiene directorio padre")?;
    fs::create_dir_all(parent).context("no se pudo crear el directorio de Codex")?;
    validate_path(path)?;
    let temporary = parent.join(format!(".airwiki-codex-{}.tmp", Uuid::new_v4()));
    let backup = parent.join(format!(".airwiki-codex-{}.bak", Uuid::new_v4()));
    let result = (|| {
        stage(path, &temporary, original.is_some(), bytes)?;
        ensure!(
            read_bytes(path)?.as_deref() == original,
            "la configuración de Codex cambió durante la operación; no se reemplazó"
        );
        if original.is_some() {
            replace_existing(path, &temporary, &backup)?;
        } else {
            // Creating the destination without replacement preserves a file
            // concurrently created by Codex after our final comparison.
            fs::hard_link(&temporary, path)
                .context("no se pudo crear la configuración de Codex sin sobrescribir archivos")?;
        }
        ensure!(
            read_bytes(path)?.as_deref() == Some(bytes),
            "la configuración de Codex cambió después de guardarla; conserva el respaldo para revisarla"
        );
        Ok(())
    })();
    let _ = fs::remove_file(&temporary);
    if result.is_ok() {
        let _ = fs::remove_file(&backup);
    }
    // On an ambiguous replacement failure keep the native backup for recovery;
    // never overwrite a concurrent edit or report an unverified save as success.
    result
}

fn stage(path: &Path, temporary: &Path, existing: bool, bytes: &[u8]) -> Result<()> {
    if existing {
        ensure!(
            !fs::metadata(path)?.permissions().readonly(),
            "la configuración de Codex es de sólo lectura"
        );
        copy_existing(path, temporary)?;
    }
    let mut options = OpenOptions::new();
    options.write(true).truncate(true);
    if !existing {
        options.create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
    }
    let mut file = options
        .open(temporary)
        .context("no se pudo preparar la configuración temporal")?;
    file.write_all(bytes)
        .context("no se pudo escribir la configuración temporal")?;
    file.sync_all()
        .context("no se pudo sincronizar la configuración temporal")
}

#[cfg(windows)]
fn copy_existing(path: &Path, temporary: &Path) -> Result<()> {
    use windows::{Win32::Storage::FileSystem::CopyFileW, core::PCWSTR};
    let source = wide_path(path)?;
    let target = wide_path(temporary)?;
    // SAFETY: both buffers are owned, NUL-terminated, contain no interior NUL,
    // and outlive this synchronous call. CopyFileW preserves Windows security
    // attributes before any user configuration is written to the staged file.
    unsafe { CopyFileW(PCWSTR(source.as_ptr()), PCWSTR(target.as_ptr()), true) }
        .context("no se pudo preparar una copia de Codex conservando sus permisos")
}

#[cfg(windows)]
fn replace_existing(path: &Path, temporary: &Path, backup: &Path) -> Result<()> {
    use windows::{
        Win32::Storage::FileSystem::{REPLACE_FILE_FLAGS, ReplaceFileW},
        core::PCWSTR,
    };
    let destination = wide_path(path)?;
    let source = wide_path(temporary)?;
    let backup = wide_path(backup)?;
    // SAFETY: owned NUL-terminated buffers stay alive for the synchronous call.
    // No ignore-ACL/merge flags: failure to preserve DACLs must fail the save.
    unsafe { ReplaceFileW(PCWSTR(destination.as_ptr()), PCWSTR(source.as_ptr()),
        PCWSTR(backup.as_ptr()), REPLACE_FILE_FLAGS(0), None, None) }
        .context("Windows no pudo reemplazar la configuración de Codex; conserva cualquier respaldo .airwiki-codex para recuperarla")
}

#[cfg(windows)]
fn wide_path(path: &Path) -> Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    let mut value: Vec<u16> = path.as_os_str().encode_wide().collect();
    ensure!(
        !value.contains(&0),
        "la ruta de Codex contiene un carácter no permitido"
    );
    value.push(0);
    Ok(value)
}

#[cfg(not(windows))]
fn copy_existing(path: &Path, temporary: &Path) -> Result<()> {
    // This backend is selected only on Windows; the portable implementation
    // exercises the same transaction and negative fixtures on other CI hosts.
    let mut source = fs::File::open(path)?;
    let mut target = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temporary)?;
    fs::set_permissions(temporary, source.metadata()?.permissions())?;
    std::io::copy(&mut source, &mut target)?;
    Ok(())
}

#[cfg(not(windows))]
fn replace_existing(path: &Path, temporary: &Path, backup: &Path) -> Result<()> {
    fs::hard_link(path, backup)?;
    fs::rename(temporary, path)?;
    if let Some(parent) = path.parent() {
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_dir() -> TempDir {
        TempDir::new_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap()
    }

    fn managed(root: &Path) -> ManagedConfiguration {
        ManagedConfiguration {
            command: root.join("bridge with spaces & %VALUE%/airwiki-mcp-bridge.exe"),
            args: vec!["--client".into(), "chatgpt-desktop".into()],
            parse_conflict: false,
        }
    }

    #[tokio::test]
    async fn connect_update_disconnect_preserve_other_servers_settings_and_comments() {
        let temp = temp_dir();
        let path = temp.path().join("config.toml");
        let original = "# personal preferences\nmodel = 'example'\n\n[mcp_servers.other]\ncommand = 'other.exe' # keep this\nargs = ['literal']\n";
        fs::write(&path, original).unwrap();
        let first = managed(temp.path());
        replace_configuration(&path, None, Some(&first))
            .await
            .unwrap();
        assert_eq!(
            read_configuration(&path).await.unwrap(),
            Some(first.clone())
        );
        let connected = fs::read_to_string(&path).unwrap();
        assert!(connected.contains(original.trim_end()));
        let mut next = first.clone();
        next.command = temp.path().join("next-bridge.exe");
        replace_configuration(&path, Some(&first), Some(&next))
            .await
            .unwrap();
        assert_eq!(read_configuration(&path).await.unwrap(), Some(next.clone()));
        replace_configuration(&path, Some(&next), None)
            .await
            .unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap().trim_end(),
            original.trim_end()
        );
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn a_first_connection_creates_a_valid_config_and_disconnect_leaves_it_readable() {
        let temp = temp_dir();
        let path = temp.path().join("custom Codex Unicode ñ/config.toml");
        let desired = managed(temp.path());
        replace_configuration(&path, None, Some(&desired))
            .await
            .unwrap();
        assert_eq!(
            read_configuration(&path).await.unwrap(),
            Some(desired.clone())
        );
        replace_configuration(&path, Some(&desired), None)
            .await
            .unwrap();
        assert_eq!(read_configuration(&path).await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_concurrent_or_foreign_entry_is_never_replaced() {
        let temp = temp_dir();
        let path = temp.path().join("config.toml");
        let original = "[mcp_servers.airwiki]\ncommand = 'foreign.exe'\nargs = []\n";
        fs::write(&path, original).unwrap();
        assert!(
            replace_configuration(&path, None, Some(&managed(temp.path())))
                .await
                .is_err()
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    }

    #[test]
    fn concurrent_file_changes_abort_commit_and_keep_the_newer_bytes() {
        let temp = temp_dir();
        let path = temp.path().join("config.toml");
        let newer = b"model = 'newer'\n";
        fs::write(&path, newer).unwrap();
        assert!(commit(&path, Some(b"model = 'old'\n"), b"model = 'attempt'\n").is_err());
        assert_eq!(fs::read(&path).unwrap(), newer);
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
        assert!(commit(&path, None, b"model = 'attempt'\n").is_err());
        assert_eq!(fs::read(&path).unwrap(), newer);
    }

    #[tokio::test]
    async fn invalid_oversized_or_nonstandard_toml_is_not_modified_or_quoted_in_errors() {
        let temp = temp_dir();
        let path = temp.path().join("config.toml");
        for original in [
            b"private_fixture = 'unterminated_secret\n".to_vec(),
            b"mcp_servers = 3\n".to_vec(),
            b"mcp_servers = { other = { command = 'other' } }\n".to_vec(),
            vec![b' '; MAX_CONFIG_BYTES + 1],
        ] {
            fs::write(&path, &original).unwrap();
            let error = replace_configuration(&path, None, Some(&managed(temp.path())))
                .await
                .unwrap_err();
            assert!(!format!("{error:#}").contains("unterminated_secret"));
            assert_eq!(fs::read(&path).unwrap(), original);
        }
    }

    #[tokio::test]
    async fn disabled_environment_bearing_and_extended_entries_are_conflicts() {
        let temp = temp_dir();
        let path = temp.path().join("config.toml");
        for suffix in [
            "enabled = false",
            "env = { TOKEN = 'synthetic' }",
            "cwd = 'elsewhere'",
            "args = [42]",
        ] {
            fs::write(
                &path,
                format!("[mcp_servers.airwiki]\ncommand = 'bridge'\n{suffix}\n"),
            )
            .unwrap();
            assert!(
                read_configuration(&path)
                    .await
                    .unwrap()
                    .unwrap()
                    .parse_conflict
            );
        }
    }

    #[tokio::test]
    async fn read_only_configuration_remains_unchanged() {
        let temp = temp_dir();
        let path = temp.path().join("config.toml");
        let original = b"model = 'example'\n";
        fs::write(&path, original).unwrap();
        let permissions = fs::metadata(&path).unwrap().permissions();
        let mut readonly = permissions.clone();
        readonly.set_readonly(true);
        fs::set_permissions(&path, readonly).unwrap();
        let result = replace_configuration(&path, None, Some(&managed(temp.path()))).await;
        fs::set_permissions(&path, permissions).unwrap();
        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
    }

    #[tokio::test]
    async fn utf8_bom_is_preserved() {
        let temp = temp_dir();
        let path = temp.path().join("config.toml");
        fs::write(&path, b"\xef\xbb\xbfmodel = 'example'\n").unwrap();
        replace_configuration(&path, None, Some(&managed(temp.path())))
            .await
            .unwrap();
        assert!(fs::read(&path).unwrap().starts_with(b"\xef\xbb\xbf"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinked_configuration_and_parent_are_rejected() {
        use std::os::unix::fs::symlink;
        let temp = temp_dir();
        let outside = temp.path().join("original.toml");
        fs::write(&outside, b"model = 'untouched'\n").unwrap();
        let link = temp.path().join("config.toml");
        symlink(&outside, &link).unwrap();
        assert!(
            replace_configuration(&link, None, Some(&managed(temp.path())))
                .await
                .is_err()
        );
        let parent = temp.path().join("linked-parent");
        symlink(temp.path(), &parent).unwrap();
        assert!(
            read_configuration(&parent.join("missing.toml"))
                .await
                .is_err()
        );
        assert_eq!(fs::read(&outside).unwrap(), b"model = 'untouched'\n");
    }

    #[cfg(windows)]
    #[test]
    fn native_replacement_retains_a_recoverable_original_backup() {
        let temp = temp_dir();
        let original = temp.path().join("original ñ.toml");
        let staged = temp.path().join("staged.toml");
        let backup = temp.path().join("backup.toml");
        fs::write(&original, b"model = 'old'\n").unwrap();
        stage(&original, &staged, true, b"model = 'new'\n").unwrap();
        replace_existing(&original, &staged, &backup).unwrap();
        assert_eq!(fs::read(&original).unwrap(), b"model = 'new'\n");
        assert_eq!(fs::read(&backup).unwrap(), b"model = 'old'\n");
        assert!(!staged.exists());
    }

    #[cfg(windows)]
    #[test]
    fn a_locked_windows_configuration_survives_failure_and_can_be_retried() {
        use std::os::windows::fs::OpenOptionsExt;
        let temp = temp_dir();
        let original = temp.path().join("original.toml");
        let staged = temp.path().join("staged.toml");
        let backup = temp.path().join("backup.toml");
        fs::write(&original, b"model = 'old'\n").unwrap();
        stage(&original, &staged, true, b"model = 'new'\n").unwrap();
        let locked = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&original)
            .unwrap();
        assert!(replace_existing(&original, &staged, &backup).is_err());
        drop(locked);
        assert_eq!(fs::read(&original).unwrap(), b"model = 'old'\n");
        replace_existing(&original, &staged, &backup).unwrap();
        assert_eq!(fs::read(&original).unwrap(), b"model = 'new'\n");
        assert_eq!(fs::read(&backup).unwrap(), b"model = 'old'\n");
    }
}
