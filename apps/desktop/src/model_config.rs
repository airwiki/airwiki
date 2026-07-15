use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use airwiki_inference::{ModelCapability, ModelProfile};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};

pub const CONFIG_SCHEMA_VERSION: u32 = 2;
pub const MODEL_CATALOG_VERSION: u32 = 1;
pub const ONBOARDING_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalePreference {
    #[default]
    System,
    Es,
    En,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LanPreference {
    #[default]
    Undecided,
    Disabled,
    Enabled,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseBehavior {
    #[default]
    Ask,
    HideToTray,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptedLicense {
    pub model_id: String,
    pub revision: String,
    pub license: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesktopConfig {
    pub schema_version: u32,
    pub catalog_version: u32,
    pub profile: ModelProfile,
    pub enabled_capabilities: BTreeSet<ModelCapability>,
    /// Catalog IDs are immutable per artifact revision; see `ModelManifest::id`.
    pub active_selection: Option<String>,
    pub pending_selection: Option<String>,
    pub accepted_licenses: Vec<AcceptedLicense>,
    pub completed_onboarding_version: Option<u32>,
    pub locale: LocalePreference,
    pub lan_preference: LanPreference,
    pub close_behavior: CloseBehavior,
    pub automatic_update_checks: bool,
}

impl Default for DesktopConfig {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            catalog_version: MODEL_CATALOG_VERSION,
            profile: ModelProfile::Automatic,
            enabled_capabilities: BTreeSet::from([ModelCapability::StructuredText]),
            active_selection: None,
            pending_selection: None,
            accepted_licenses: Vec::new(),
            completed_onboarding_version: None,
            locale: LocalePreference::System,
            lan_preference: LanPreference::Undecided,
            close_behavior: CloseBehavior::Ask,
            automatic_update_checks: false,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DesktopConfigV1 {
    schema_version: u32,
    catalog_version: u32,
    profile: ModelProfile,
    enabled_capabilities: BTreeSet<ModelCapability>,
    active_selection: Option<String>,
    pending_selection: Option<String>,
    accepted_licenses: Vec<AcceptedLicense>,
}

impl DesktopConfigV1 {
    fn migrate(self) -> Result<DesktopConfig> {
        if self.schema_version != 1 {
            bail!("la configuración no corresponde al esquema 1");
        }
        validate_model_state(self.catalog_version, &self.enabled_capabilities)?;
        Ok(DesktopConfig {
            schema_version: CONFIG_SCHEMA_VERSION,
            catalog_version: self.catalog_version,
            profile: self.profile,
            enabled_capabilities: self.enabled_capabilities,
            active_selection: self.active_selection,
            pending_selection: self.pending_selection,
            accepted_licenses: self.accepted_licenses,
            // Version 1 predates the wizard and already ran LAN services. Keep
            // that behavior, while leaving every new remote action opt-in.
            completed_onboarding_version: Some(ONBOARDING_VERSION),
            locale: LocalePreference::System,
            lan_preference: LanPreference::Enabled,
            close_behavior: CloseBehavior::Ask,
            automatic_update_checks: false,
        })
    }
}

#[derive(Debug)]
pub struct ConfigLoad {
    pub config: DesktopConfig,
    pub warning: Option<String>,
}

impl DesktopConfig {
    pub fn load_or_default(path: &Path) -> Result<ConfigLoad> {
        recover_interrupted_replace(path)?;
        if !path.exists() {
            return Ok(ConfigLoad {
                config: Self::default(),
                warning: None,
            });
        }

        let bytes = fs::read(path)
            .with_context(|| format!("no se pudo leer la configuración {}", path.display()))?;
        let value = match serde_json::from_slice::<serde_json::Value>(&bytes) {
            Ok(value) => value,
            Err(error) => return quarantine_invalid(path, error.to_string()),
        };
        let schema_version = value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .and_then(|version| u32::try_from(version).ok());
        let catalog_version = value
            .get("catalog_version")
            .and_then(serde_json::Value::as_u64)
            .and_then(|version| u32::try_from(version).ok());

        // Reject future state before deserializing fields unknown to this build,
        // so the original file remains byte-for-byte intact.
        if let Some(schema_version) = schema_version
            && schema_version > CONFIG_SCHEMA_VERSION
        {
            bail!(
                "la configuración usa el esquema {} pero esta aplicación solo admite hasta {}",
                schema_version,
                CONFIG_SCHEMA_VERSION
            );
        }
        if let Some(catalog_version) = catalog_version
            && catalog_version > MODEL_CATALOG_VERSION
        {
            bail!(
                "la configuración usa el catálogo {} pero esta aplicación solo admite hasta {}",
                catalog_version,
                MODEL_CATALOG_VERSION
            );
        }

        match schema_version {
            Some(1) => {
                let migrated = serde_json::from_value::<DesktopConfigV1>(value)
                    .map_err(anyhow::Error::from)
                    .and_then(DesktopConfigV1::migrate);
                match migrated {
                    Ok(config) => {
                        config.save_atomic(path)?;
                        Ok(ConfigLoad {
                            config,
                            warning: None,
                        })
                    }
                    Err(error) => quarantine_invalid(path, error.to_string()),
                }
            }
            Some(CONFIG_SCHEMA_VERSION) => match serde_json::from_value::<Self>(value) {
                Ok(config) => match config.validate() {
                    Ok(()) => Ok(ConfigLoad {
                        config,
                        warning: None,
                    }),
                    Err(error) => quarantine_invalid(path, error.to_string()),
                },
                Err(error) => quarantine_invalid(path, error.to_string()),
            },
            _ => quarantine_invalid(path, "versión de esquema ausente o inválida".to_owned()),
        }
    }

    pub fn save_atomic(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let parent = path
            .parent()
            .context("la ruta de configuración no tiene directorio padre")?;
        fs::create_dir_all(parent)?;
        let temporary = path.with_extension(format!("json.{}.tmp", std::process::id()));
        let bytes = serde_json::to_vec_pretty(self)?;

        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        drop(file);

        replace_atomically(&temporary, path)?;
        sync_parent(parent);
        Ok(())
    }

    pub fn accepts(&self, model_id: &str, revision: &str) -> bool {
        self.accepted_licenses
            .iter()
            .any(|accepted| accepted.model_id == model_id && accepted.revision == revision)
    }

    pub fn accept_license(&mut self, model_id: &str, revision: &str, license: &str) {
        self.accepted_licenses
            .retain(|accepted| !(accepted.model_id == model_id && accepted.revision == revision));
        self.accepted_licenses.push(AcceptedLicense {
            model_id: model_id.to_owned(),
            revision: revision.to_owned(),
            license: license.to_owned(),
        });
        self.accepted_licenses.sort_by(|left, right| {
            (&left.model_id, &left.revision).cmp(&(&right.model_id, &right.revision))
        });
    }

    fn validate(&self) -> Result<()> {
        if self.schema_version > CONFIG_SCHEMA_VERSION {
            bail!(
                "la configuración usa el esquema {} pero esta aplicación solo admite hasta {}",
                self.schema_version,
                CONFIG_SCHEMA_VERSION
            );
        }
        if self.schema_version != CONFIG_SCHEMA_VERSION {
            bail!(
                "la configuración debe migrarse al esquema {} antes de guardarse",
                CONFIG_SCHEMA_VERSION
            );
        }
        validate_model_state(self.catalog_version, &self.enabled_capabilities)
    }
}

fn validate_model_state(
    catalog_version: u32,
    enabled_capabilities: &BTreeSet<ModelCapability>,
) -> Result<()> {
    if catalog_version > MODEL_CATALOG_VERSION {
        bail!(
            "la configuración usa el catálogo {catalog_version} pero esta aplicación solo admite hasta {MODEL_CATALOG_VERSION}"
        );
    }
    if catalog_version == 0 {
        bail!("la versión cero del catálogo de modelos no es válida");
    }
    if !enabled_capabilities.contains(&ModelCapability::StructuredText) {
        bail!("structured_text es una capacidad obligatoria");
    }
    Ok(())
}

fn quarantine_invalid(path: &Path, reason: String) -> Result<ConfigLoad> {
    let quarantine = corrupt_path(path);
    fs::rename(path, &quarantine).with_context(|| {
        format!(
            "la configuración es inválida y no se pudo preservar en {}",
            quarantine.display()
        )
    })?;
    Ok(ConfigLoad {
        config: DesktopConfig::default(),
        warning: Some(format!(
            "La configuración inválida se preservó en {}: {reason}",
            quarantine.display()
        )),
    })
}

fn corrupt_path(path: &Path) -> PathBuf {
    let timestamp = Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config.json");
    path.with_file_name(format!("{filename}.corrupt-{timestamp}"))
}

fn previous_path(path: &Path) -> PathBuf {
    path.with_extension("json.previous")
}

fn recover_interrupted_replace(path: &Path) -> Result<()> {
    let previous = previous_path(path);
    if !path.exists() && previous.is_file() {
        fs::rename(&previous, path)?;
    } else if path.is_file() && previous.exists() {
        fs::remove_file(previous)?;
    }
    Ok(())
}

#[cfg(unix)]
fn replace_atomically(temporary: &Path, destination: &Path) -> Result<()> {
    fs::rename(temporary, destination)?;
    Ok(())
}

#[cfg(windows)]
fn replace_atomically(temporary: &Path, destination: &Path) -> Result<()> {
    let previous = previous_path(destination);
    if previous.exists() {
        fs::remove_file(&previous)?;
    }
    if destination.exists() {
        fs::rename(destination, &previous)?;
    }
    if let Err(error) = fs::rename(temporary, destination) {
        if previous.exists() {
            fs::rename(&previous, destination).ok();
        }
        return Err(error.into());
    }
    if previous.exists() {
        fs::remove_file(previous)?;
    }
    Ok(())
}

fn sync_parent(parent: &Path) {
    if let Ok(directory) = File::open(parent) {
        directory.sync_all().ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_install_starts_private_and_without_remote_checks() {
        let directory = tempfile::tempdir().unwrap();
        let loaded = DesktopConfig::load_or_default(&directory.path().join("config.json")).unwrap();

        assert_eq!(loaded.config.completed_onboarding_version, None);
        assert_eq!(loaded.config.lan_preference, LanPreference::Undecided);
        assert!(!loaded.config.automatic_update_checks);
    }

    #[test]
    fn version_one_migration_preserves_model_state_and_existing_lan_behavior() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.json");
        let legacy = serde_json::json!({
            "schema_version": 1,
            "catalog_version": 1,
            "profile": "quality",
            "enabled_capabilities": ["structured_text"],
            "active_selection": "qwen3-1.7b-q8",
            "pending_selection": "gemma-4-e4b-q4",
            "accepted_licenses": [{
                "model_id": "qwen3-1.7b-q8",
                "revision": "synthetic-revision",
                "license": "Apache-2.0"
            }]
        });
        fs::write(&path, serde_json::to_vec_pretty(&legacy).unwrap()).unwrap();

        let loaded = DesktopConfig::load_or_default(&path).unwrap();

        assert_eq!(loaded.config.schema_version, CONFIG_SCHEMA_VERSION);
        assert_eq!(loaded.config.profile, ModelProfile::Quality);
        assert_eq!(
            loaded.config.active_selection.as_deref(),
            Some("qwen3-1.7b-q8")
        );
        assert_eq!(
            loaded.config.pending_selection.as_deref(),
            Some("gemma-4-e4b-q4")
        );
        assert!(loaded.config.accepts("qwen3-1.7b-q8", "synthetic-revision"));
        assert_eq!(
            loaded.config.completed_onboarding_version,
            Some(ONBOARDING_VERSION)
        );
        assert_eq!(loaded.config.lan_preference, LanPreference::Enabled);
        assert_eq!(loaded.config.close_behavior, CloseBehavior::Ask);
        assert!(!loaded.config.automatic_update_checks);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&fs::read(path).unwrap()).unwrap()["schema_version"],
            CONFIG_SCHEMA_VERSION
        );
    }

    #[test]
    fn configuration_round_trips_atomically() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.json");
        let mut config = DesktopConfig {
            profile: ModelProfile::Quality,
            pending_selection: Some("gemma-4-e4b-q4".into()),
            ..DesktopConfig::default()
        };
        config.accept_license("gemma-4-e4b-q4", "revision", "Apache-2.0");
        config.save_atomic(&path).unwrap();

        let loaded = DesktopConfig::load_or_default(&path).unwrap();
        assert_eq!(loaded.config, config);
        assert!(loaded.warning.is_none());
    }

    #[test]
    fn corrupt_configuration_is_quarantined() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.json");
        fs::write(&path, b"{not-json").unwrap();

        let loaded = DesktopConfig::load_or_default(&path).unwrap();
        assert_eq!(loaded.config, DesktopConfig::default());
        assert!(loaded.warning.is_some());
        assert!(!path.exists());
        assert_eq!(
            fs::read_dir(directory.path())
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().contains(".corrupt-"))
                .count(),
            1
        );
    }

    #[test]
    fn future_schema_is_rejected_without_rewriting() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.json");
        let mut value = serde_json::to_value(DesktopConfig::default()).unwrap();
        value["schema_version"] = serde_json::json!(CONFIG_SCHEMA_VERSION + 1);
        let bytes = serde_json::to_vec(&value).unwrap();
        fs::write(&path, &bytes).unwrap();

        let error = DesktopConfig::load_or_default(&path)
            .unwrap_err()
            .to_string();
        assert!(error.contains("solo admite"));
        assert_eq!(fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn future_catalog_is_rejected_without_rewriting() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.json");
        let mut value = serde_json::to_value(DesktopConfig::default()).unwrap();
        value["catalog_version"] = serde_json::json!(MODEL_CATALOG_VERSION + 1);
        let bytes = serde_json::to_vec(&value).unwrap();
        fs::write(&path, &bytes).unwrap();

        let error = DesktopConfig::load_or_default(&path)
            .unwrap_err()
            .to_string();
        assert!(error.contains("catálogo"));
        assert_eq!(fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn invalid_current_configuration_is_quarantined() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.json");
        let mut config = DesktopConfig::default();
        config.enabled_capabilities.clear();
        fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();

        let loaded = DesktopConfig::load_or_default(&path).unwrap();
        assert_eq!(loaded.config, DesktopConfig::default());
        assert!(loaded.warning.unwrap().contains("structured_text"));
        assert!(!path.exists());
    }
}
