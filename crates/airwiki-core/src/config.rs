use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use directories::ProjectDirs;
use uuid::Uuid;

/// All mutable application state lives below this platform-appropriate root.
#[derive(Debug, Clone)]
pub struct AppPaths {
    pub root: PathBuf,
    pub database: PathBuf,
    pub vaults: PathBuf,
    pub models: PathBuf,
    pub cache: PathBuf,
    pub logs: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Result<Self> {
        let dirs = ProjectDirs::from("io.github", "airwiki", "AirWiki")
            .context("the operating system did not provide an application data directory")?;
        Ok(Self::at(dirs.data_local_dir()))
    }

    pub fn at(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref().to_path_buf();
        Self {
            database: root.join("airwiki.sqlite3"),
            vaults: root.join("vaults"),
            models: root.join("models"),
            cache: root.join("cache"),
            logs: root.join("logs"),
            root,
        }
    }

    pub fn ensure(&self) -> Result<()> {
        for path in [
            &self.root,
            &self.vaults,
            &self.models,
            &self.cache,
            &self.logs,
        ] {
            fs::create_dir_all(path)
                .with_context(|| format!("could not create {}", path.display()))?;
        }
        Ok(())
    }

    pub fn collection(&self, collection_id: Uuid) -> CollectionPaths {
        CollectionPaths::at(self.vaults.join(collection_id.to_string()))
    }
}

#[derive(Debug, Clone)]
pub struct CollectionPaths {
    pub root: PathBuf,
    pub concepts: PathBuf,
    pub index: PathBuf,
    pub log: PathBuf,
}

impl CollectionPaths {
    pub fn at(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref().to_path_buf();
        Self {
            concepts: root.join("concepts"),
            index: root.join("index.md"),
            log: root.join("log.md"),
            root,
        }
    }

    pub fn ensure(&self) -> Result<()> {
        fs::create_dir_all(&self.concepts)
            .with_context(|| format!("could not create {}", self.concepts.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_expected_layout() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::at(temp.path().join("app"));
        paths.ensure().unwrap();
        let collection = paths.collection(Uuid::nil());
        collection.ensure().unwrap();
        assert!(paths.models.is_dir());
        assert!(collection.concepts.is_dir());
    }
}
