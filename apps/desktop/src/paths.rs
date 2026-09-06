use std::path::PathBuf;

use anyhow::{Context, Result};
#[cfg(not(feature = "e2e"))]
use directories::ProjectDirs;

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub data: PathBuf,
    pub database: PathBuf,
    pub logs: PathBuf,
    pub config: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Result<Self> {
        #[cfg(feature = "e2e")]
        {
            Self::e2e_paths(std::env::var_os("AIRWIKI_E2E_DATA_ROOT").as_deref())
        }
        #[cfg(not(feature = "e2e"))]
        {
            let project = ProjectDirs::from("io.github", "airwiki", "AirWiki")
                .context("the operating system did not expose an application data directory")?;
            let data = project.data_local_dir().to_path_buf();
            let config_dir = project.config_dir();
            std::fs::create_dir_all(&data)?;
            std::fs::create_dir_all(config_dir)?;
            Ok(Self {
                database: data.join("airwiki.sqlite3"),
                logs: data.join("logs"),
                config: config_dir.join("config.json"),
                data,
            })
        }
    }

    #[cfg(feature = "e2e")]
    fn e2e_paths(value: Option<&std::ffi::OsStr>) -> Result<Self> {
        let value = value.context("E2E builds require an isolated temporary data root")?;
        let root = std::path::Path::new(value);
        anyhow::ensure!(root.is_absolute(), "the E2E data root must be absolute");
        let root = root
            .canonicalize()
            .context("the E2E data root must already exist")?;
        let temporary = std::env::temp_dir().canonicalize()?;
        anyhow::ensure!(
            root.parent() == Some(temporary.as_path())
                && root
                    .file_name()
                    .and_then(std::ffi::OsStr::to_str)
                    .is_some_and(|name| name.starts_with("airwiki-e2e-")),
            "E2E builds require an isolated temporary data root"
        );
        let data = root.join("data");
        let config = root.join("config");
        // Check existing directories before creating anything. A retained QA
        // profile may be reused, but a linked data/config directory is unsafe.
        for directory in [&data, &config] {
            match std::fs::symlink_metadata(directory) {
                Ok(metadata) => anyhow::ensure!(
                    metadata.is_dir()
                        && !metadata.file_type().is_symlink()
                        && directory.canonicalize()? == *directory,
                    "E2E profile directories must remain inside the temporary root"
                ),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        std::fs::create_dir_all(&data)?;
        std::fs::create_dir_all(&config)?;
        Ok(Self {
            database: data.join("airwiki.sqlite3"),
            logs: data.join("logs"),
            config: config.join("config.json"),
            data,
        })
    }

    pub fn bundled_llama_server(&self) -> Option<PathBuf> {
        let mut candidates = Vec::new();
        if let Ok(executable) = std::env::current_exe()
            && let Some(parent) = executable.parent()
        {
            #[cfg(target_os = "macos")]
            candidates.push(parent.join("../Resources/llama/llama-b9946/llama-server"));
            #[cfg(target_os = "windows")]
            candidates.push(parent.join("llama/llama-server.exe"));
        }
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        #[cfg(target_os = "macos")]
        candidates.push(workspace.join("resources/llama/macos-aarch64/llama-b9946/llama-server"));
        #[cfg(target_os = "windows")]
        candidates.push(workspace.join("resources/llama/windows-x64/llama-server.exe"));
        candidates.into_iter().find(|path| path.is_file())
    }
}

#[cfg(all(test, feature = "e2e"))]
mod tests {
    use super::*;

    #[test]
    fn e2e_paths_require_an_existing_named_temporary_root() -> Result<()> {
        assert!(AppPaths::e2e_paths(None).is_err());
        assert!(AppPaths::e2e_paths(Some(std::ffi::OsStr::new("relative"))).is_err());
        let unrelated = tempfile::tempdir()?;
        assert!(AppPaths::e2e_paths(Some(unrelated.path().as_os_str())).is_err());
        assert!(!unrelated.path().join("data").exists());
        let root = tempfile::Builder::new().prefix("airwiki-e2e-").tempdir()?;
        let paths = AppPaths::e2e_paths(Some(root.path().as_os_str()))?;
        assert_eq!(paths.data, root.path().canonicalize()?.join("data"));
        std::fs::write(&paths.config, "synthetic preserved config")?;
        let reopened = AppPaths::e2e_paths(Some(root.path().as_os_str()))?;
        assert_eq!(
            std::fs::read(reopened.config)?,
            b"synthetic preserved config"
        );
        Ok(())
    }

    #[test]
    fn e2e_paths_reject_nested_roots_before_creating_a_profile() -> Result<()> {
        let parent = tempfile::tempdir()?;
        let root = parent.path().join("airwiki-e2e-nested");
        std::fs::create_dir(&root)?;
        assert!(AppPaths::e2e_paths(Some(root.as_os_str())).is_err());
        assert!(!root.join("data").exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn e2e_paths_reject_linked_profile_directories_before_writing() -> Result<()> {
        let root = tempfile::Builder::new().prefix("airwiki-e2e-").tempdir()?;
        let unrelated = tempfile::tempdir()?;
        std::os::unix::fs::symlink(unrelated.path(), root.path().join("data"))?;
        assert!(AppPaths::e2e_paths(Some(root.path().as_os_str())).is_err());
        assert!(!root.path().join("config").exists());
        assert_eq!(std::fs::read_dir(unrelated.path())?.count(), 0);
        Ok(())
    }
}
