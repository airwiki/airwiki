use std::path::PathBuf;

use anyhow::{Context, Result};
use directories::ProjectDirs;

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub data: PathBuf,
    pub database: PathBuf,
    pub vaults: PathBuf,
    pub logs: PathBuf,
    pub config: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Result<Self> {
        let project = ProjectDirs::from("io.github", "airwiki", "AirWiki")
            .context("the operating system did not expose an application data directory")?;
        let data = project.data_local_dir().to_path_buf();
        let config_dir = project.config_dir();
        std::fs::create_dir_all(&data)?;
        std::fs::create_dir_all(config_dir)?;
        Ok(Self {
            database: data.join("airwiki.sqlite3"),
            vaults: data.join("vaults"),
            logs: data.join("logs"),
            config: config_dir.join("config.json"),
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
