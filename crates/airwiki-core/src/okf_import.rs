use std::collections::{BTreeSet, HashSet};
use std::fs::{self, File};
use std::io::{Read, Seek};
use std::path::{Component, Path, PathBuf};

use airwiki_types::{
    ActorId, AttestedArtifact, AttestedComputationContract, AttestedExecutor, AttestedParameter,
    ConceptAssurance, FreshnessState, OkfCompatibility, OkfWarning, OkfWarningCode, TrustTier,
};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, NaiveDate, Utc};
use serde_yaml::Value as YamlValue;
use sha2::Digest;
use unicode_normalization::UnicodeNormalization;
use walkdir::WalkDir;
use zip::ZipArchive;

#[cfg(windows)]
use std::os::windows::fs::MetadataExt;

#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;

const MAX_ENTRIES: usize = 5_000;
const MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
const MAX_CONCEPT_BYTES: u64 = 1024 * 1024;
const MAX_RESOURCE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_FRONTMATTER_BYTES: usize = 64 * 1024;
const MAX_YAML_DEPTH: usize = 24;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OkfImportReport {
    pub entry_count: usize,
    pub concept_count: usize,
    pub uncompressed_bytes: u64,
    pub declared_okf_version: Option<String>,
    pub compatibility: OkfCompatibility,
    pub warnings: Vec<OkfWarning>,
    pub concepts: Vec<OkfImportedConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OkfImportedConcept {
    pub logical_path: String,
    pub concept_type: String,
    pub title: String,
    pub description: String,
    pub tags: Vec<String>,
    pub lifecycle_status: String,
    pub generated: Option<YamlValue>,
    pub verified: Option<YamlValue>,
    pub sources: Option<YamlValue>,
    pub stale_after: Option<String>,
    pub version: Option<String>,
    pub unknown_frontmatter: YamlValue,
    pub attested_computation: Option<AttestedComputationContract>,
    pub fingerprint: String,
    pub search_text: String,
    pub assurance: ConceptAssurance,
    pub warnings: Vec<OkfWarning>,
}

#[derive(Debug, Default)]
pub struct OkfImportValidator;

impl OkfImportValidator {
    pub fn validate_path(path: &Path) -> Result<OkfImportReport> {
        let metadata = fs::symlink_metadata(path)
            .with_context(|| format!("could not inspect OKF import {}", path.display()))?;
        if metadata_is_link_or_reparse(&metadata) {
            bail!("OKF import root must not be a symbolic link");
        }
        if metadata.is_dir() {
            Self::validate_directory(path)
        } else if metadata.is_file()
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
        {
            Self::validate_zip(File::open(path).context("could not open OKF ZIP")?)
        } else {
            bail!("OKF import must be a directory or ZIP archive");
        }
    }

    pub fn validate_directory(root: &Path) -> Result<OkfImportReport> {
        let mut state = ValidationState::default();
        for entry in WalkDir::new(root).follow_links(false) {
            let entry = entry.context("OKF directory traversal failed")?;
            if entry.path() == root {
                continue;
            }
            let metadata = fs::symlink_metadata(entry.path())?;
            if metadata_is_link_or_reparse(&metadata) {
                bail!("OKF imports must not contain symbolic links");
            }
            if metadata.is_dir() {
                let relative = entry
                    .path()
                    .strip_prefix(root)
                    .context("OKF directory escaped its root")?;
                state.accept_directory(validated_relative_path(relative)?)?;
                continue;
            }
            if !metadata.is_file() {
                bail!("OKF imports may contain only regular files and directories");
            }
            let relative = entry
                .path()
                .strip_prefix(root)
                .context("OKF entry escaped its root")?;
            let logical_path = validated_relative_path(relative)?;
            if metadata.len() > file_limit(&logical_path) {
                bail!("OKF import entry exceeds its size limit");
            }
            state.accept(logical_path, &fs::read(entry.path())?)?;
        }
        state.finish()
    }

    pub fn validate_zip<R: Read + Seek>(reader: R) -> Result<OkfImportReport> {
        let mut archive = ZipArchive::new(reader).context("invalid OKF ZIP archive")?;
        let mut state = ValidationState::default();
        let mut names = HashSet::new();
        for index in 0..archive.len() {
            let mut entry = archive
                .by_index(index)
                .context("could not inspect ZIP entry")?;
            let enclosed = entry
                .enclosed_name()
                .context("ZIP entry contains an absolute path or traversal")?;
            let logical_path = validated_relative_path(&enclosed)?;
            if !names.insert(portable_path_key(&logical_path)) {
                bail!("ZIP contains duplicate or non-portable colliding entries");
            }
            if entry.is_dir() {
                state.accept_directory(logical_path)?;
                continue;
            }
            if entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 != 0o100000)
            {
                bail!("ZIP contains a non-regular entry");
            }
            let limit = file_limit(&logical_path);
            if entry.size() > limit {
                bail!("ZIP entry exceeds its size limit");
            }
            let capacity = usize::try_from(entry.size()).context("ZIP entry is too large")?;
            let mut bytes = Vec::with_capacity(capacity);
            entry
                .by_ref()
                .take(limit.saturating_add(1))
                .read_to_end(&mut bytes)
                .context("could not read ZIP entry")?;
            if u64::try_from(bytes.len())? > limit {
                bail!("ZIP entry exceeds its size limit");
            }
            state.accept(logical_path, &bytes)?;
        }
        state.finish()
    }

    pub fn materialize_path(source: &Path, staging: &Path) -> Result<OkfImportReport> {
        if staging.exists() {
            bail!("OKF import staging directory already exists");
        }
        let expected = Self::validate_path(source)?;
        fs::create_dir(staging).context("could not create OKF import staging directory")?;
        let materialized = (|| {
            if fs::symlink_metadata(source)?.is_dir() {
                copy_directory(source, staging)?;
            } else {
                extract_zip(source, staging)?;
            }
            let actual = Self::validate_directory(staging)?;
            if actual != expected {
                bail!("OKF import changed while it was being copied");
            }
            Ok(actual)
        })();
        if materialized.is_err() {
            let _ = fs::remove_dir_all(staging);
        }
        materialized
    }
}

pub(crate) fn append_human_verification(
    markdown: &str,
    actor: &ActorId,
    verified_at: DateTime<Utc>,
) -> Result<String> {
    if !actor.is_human() {
        bail!("OKF human verification requires a human actor");
    }
    let parsed = split_frontmatter(markdown)?;
    let mut yaml = parsed.yaml.context("OKF concept is missing frontmatter")?;
    let mapping = yaml
        .as_mapping_mut()
        .context("OKF frontmatter must be a mapping")?;
    let type_key = YamlValue::String("type".to_owned());
    if string_field(mapping, "type").is_none_or(|value| value.trim().is_empty()) {
        bail!("OKF concept type is required");
    }
    let verified_key = YamlValue::String("verified".to_owned());
    let mut verifications = match mapping.remove(&verified_key) {
        None => Vec::new(),
        Some(value) if verifications_are_well_formed(&value) => match value {
            YamlValue::Sequence(values) => values,
            value @ YamlValue::Mapping(_) => vec![value],
            _ => Vec::new(),
        },
        Some(_) => bail!("existing OKF verification metadata is invalid"),
    };
    let verification = serde_yaml::to_value(serde_json::json!({
        "by": actor.as_str(),
        "at": verified_at.to_rfc3339(),
    }))?;
    verifications.push(verification);
    mapping.insert(verified_key, YamlValue::Sequence(verifications));
    if !mapping.contains_key(&type_key) {
        bail!("OKF concept type is required");
    }
    let frontmatter = serde_yaml::to_string(&yaml)?;
    Ok(format!("---\n{frontmatter}---\n{}", parsed.body))
}

fn copy_directory(source: &Path, staging: &Path) -> Result<()> {
    for entry in WalkDir::new(source).follow_links(false) {
        let entry = entry.context("OKF directory traversal failed while copying")?;
        if entry.path() == source {
            continue;
        }
        let relative = entry.path().strip_prefix(source)?;
        let destination = staging.join(validated_relative_path(relative)?);
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata_is_link_or_reparse(&metadata) {
            bail!("OKF import changed to a link while being copied");
        }
        if metadata.is_dir() {
            fs::create_dir_all(&destination)?;
        } else if metadata.is_file() {
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(entry.path(), &destination)?;
        } else {
            bail!("OKF import contains a non-regular entry");
        }
    }
    Ok(())
}

fn extract_zip(source: &Path, staging: &Path) -> Result<()> {
    let mut archive = ZipArchive::new(File::open(source)?).context("invalid OKF ZIP archive")?;
    let mut names = HashSet::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let enclosed = entry
            .enclosed_name()
            .context("ZIP entry contains traversal")?;
        let logical_path = validated_relative_path(&enclosed)?;
        if !names.insert(portable_path_key(&logical_path)) {
            bail!("ZIP contains duplicate or non-portable colliding entries");
        }
        let destination = staging.join(&logical_path);
        if entry.is_dir() {
            fs::create_dir_all(&destination)?;
            continue;
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 != 0o100000)
        {
            bail!("ZIP contains a non-regular entry");
        }
        if entry.size() > file_limit(&logical_path) {
            bail!("ZIP entry exceeds its size limit");
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = File::create(&destination)?;
        std::io::copy(
            &mut entry.by_ref().take(file_limit(&logical_path)),
            &mut output,
        )?;
    }
    Ok(())
}

fn metadata_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    false
}

#[derive(Default)]
struct ValidationState {
    entries: usize,
    bytes: u64,
    root_version: Option<String>,
    paths: BTreeSet<String>,
    portable_paths: HashSet<String>,
    portable_directories: HashSet<String>,
    links: Vec<(String, String)>,
    concepts: Vec<OkfImportedConcept>,
    warnings: Vec<OkfWarning>,
}

impl ValidationState {
    fn accept_directory(&mut self, path: String) -> Result<()> {
        self.count_entry()?;
        let key = portable_path_key(&path);
        if self.portable_paths.contains(&key) {
            bail!("OKF import contains a file and directory path collision");
        }
        self.portable_directories.insert(key);
        self.accept_directory_ancestors(&path)
    }

    fn accept(&mut self, path: String, bytes: &[u8]) -> Result<()> {
        self.count_entry()?;
        self.accept_directory_ancestors(&path)?;
        self.bytes = self
            .bytes
            .checked_add(u64::try_from(bytes.len())?)
            .context("OKF import size overflow")?;
        if self.bytes > MAX_TOTAL_BYTES {
            bail!("OKF import exceeds the uncompressed size limit");
        }
        let portable_key = portable_path_key(&path);
        if self.portable_directories.contains(&portable_key)
            || !self.paths.insert(path.clone())
            || !self.portable_paths.insert(portable_key)
        {
            bail!("OKF import contains duplicate or non-portable colliding paths");
        }
        if !path.to_ascii_lowercase().ends_with(".md") {
            return Ok(());
        }
        let markdown = std::str::from_utf8(bytes).context("OKF Markdown must be UTF-8")?;
        let filename = path.rsplit('/').next().unwrap_or(path.as_str());
        if filename.eq_ignore_ascii_case("index.md") {
            self.validate_index(&path, markdown)?;
        } else if filename.eq_ignore_ascii_case("log.md") {
            validate_log(markdown)?;
        } else {
            self.concepts
                .push(parse_concept(&path, markdown, Utc::now())?);
        }
        self.links.extend(
            markdown_links(markdown)
                .into_iter()
                .map(|target| (path.clone(), target)),
        );
        Ok(())
    }

    fn count_entry(&mut self) -> Result<()> {
        self.entries = self.entries.saturating_add(1);
        if self.entries > MAX_ENTRIES {
            bail!("OKF import contains too many entries");
        }
        Ok(())
    }

    fn accept_directory_ancestors(&mut self, path: &str) -> Result<()> {
        let mut ancestor = PathBuf::new();
        let components = Path::new(path).components().collect::<Vec<_>>();
        for component in components.iter().take(components.len().saturating_sub(1)) {
            ancestor.push(component.as_os_str());
            let logical = validated_relative_path(&ancestor)?;
            let key = portable_path_key(&logical);
            if self.portable_paths.contains(&key) {
                bail!("OKF import contains a file and directory path collision");
            }
            self.portable_directories.insert(key);
        }
        Ok(())
    }

    fn validate_index(&mut self, path: &str, markdown: &str) -> Result<()> {
        let parsed = split_frontmatter(markdown)?;
        validate_index_body(parsed.body)?;
        if path.contains('/') {
            if parsed.yaml.is_some() {
                bail!("nested OKF index.md must not contain frontmatter");
            }
            return Ok(());
        }
        let Some(yaml) = parsed.yaml else {
            return Ok(());
        };
        let mapping = yaml
            .as_mapping()
            .context("root index frontmatter must be a mapping")?;
        if mapping
            .keys()
            .any(|key| key.as_str() != Some("okf_version"))
        {
            bail!("root OKF index frontmatter may contain only okf_version");
        }
        if let Some(value) = mapping.get(YamlValue::String("okf_version".to_owned())) {
            self.root_version = value
                .as_str()
                .map(str::trim)
                .filter(|version| !version.is_empty())
                .map(ToOwned::to_owned);
            if self.root_version.is_none() {
                self.warnings.push(warning(
                    OkfWarningCode::InvalidOptionalMetadata,
                    path,
                    Some("okf_version"),
                ));
            }
        }
        Ok(())
    }

    fn finish(self) -> Result<OkfImportReport> {
        let saw_legacy_signal = self.concepts.iter().any(|concept| {
            concept.warnings.iter().any(|warning| {
                matches!(
                    warning.code,
                    OkfWarningCode::LegacyTimestamp | OkfWarningCode::LegacyCitations
                )
            })
        });
        let saw_v02_signal = self.concepts.iter().any(concept_has_v02_signal);
        let compatibility = match self.root_version.as_deref() {
            Some("0.2") => OkfCompatibility::DeclaredV02,
            Some("0.1") => OkfCompatibility::LegacyV01,
            Some(version) => OkfCompatibility::FutureRestricted {
                declared_version: version.to_owned(),
            },
            None if saw_legacy_signal && !saw_v02_signal => OkfCompatibility::LegacyV01,
            None => OkfCompatibility::UndeclaredV02Compatible,
        };
        let mut warnings: Vec<_> = self
            .links
            .into_iter()
            .filter(|(_, target)| is_relative_markdown_target(target))
            .filter_map(|(source, target)| {
                resolve_relative_target(&source, &target)
                    .filter(|resolved| !self.paths.contains(resolved))
                    .map(|_| warning(OkfWarningCode::BrokenLink, source, None))
            })
            .collect();
        warnings.extend(self.warnings);
        warnings.extend(
            self.concepts
                .iter()
                .flat_map(|concept| concept.warnings.clone()),
        );
        Ok(OkfImportReport {
            entry_count: self.entries,
            concept_count: self.concepts.len(),
            uncompressed_bytes: self.bytes,
            declared_okf_version: self.root_version,
            compatibility,
            warnings,
            concepts: self.concepts,
        })
    }
}

struct ParsedMarkdown<'a> {
    yaml: Option<YamlValue>,
    body: &'a str,
}

fn split_frontmatter(markdown: &str) -> Result<ParsedMarkdown<'_>> {
    let line_end = if markdown.starts_with("---\r\n") {
        "\r\n"
    } else if markdown.starts_with("---\n") {
        "\n"
    } else {
        return Ok(ParsedMarkdown {
            yaml: None,
            body: markdown,
        });
    };
    let opening = 3 + line_end.len();
    let rest = &markdown[opening..];
    let delimiter = format!("{line_end}---{line_end}");
    let end = rest
        .find(&delimiter)
        .or_else(|| {
            rest.strip_suffix(&format!("{line_end}---"))
                .map(|body| body.len())
        })
        .context("frontmatter is not terminated with an exact delimiter")?;
    if end > MAX_FRONTMATTER_BYTES {
        bail!("frontmatter exceeds 64 KiB");
    }
    let yaml: YamlValue = serde_yaml::from_str(&rest[..end]).context("invalid YAML frontmatter")?;
    if yaml_depth(&yaml, 0) > MAX_YAML_DEPTH {
        bail!("YAML nesting is too deep");
    }
    let body_start = if rest[end..].starts_with(&delimiter) {
        end + delimiter.len()
    } else {
        rest.len()
    };
    Ok(ParsedMarkdown {
        yaml: Some(yaml),
        body: &rest[body_start..],
    })
}

fn parse_concept(path: &str, markdown: &str, now: DateTime<Utc>) -> Result<OkfImportedConcept> {
    let parsed = split_frontmatter(markdown)?;
    let yaml = parsed.yaml.context("OKF concept is missing frontmatter")?;
    let mapping = yaml
        .as_mapping()
        .context("OKF frontmatter must be a mapping")?;
    let concept_type = string_field(mapping, "type").map(str::trim);
    if concept_type.is_none_or(str::is_empty) {
        bail!("OKF concept type is required");
    }
    let title = string_field(mapping, "title")
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            path.rsplit('/')
                .next()
                .unwrap_or(path)
                .trim_end_matches(".md")
                .to_owned()
        });
    let description = string_field(mapping, "description")
        .unwrap_or_default()
        .to_owned();
    let tags = mapping
        .get(YamlValue::String("tags".to_owned()))
        .and_then(YamlValue::as_sequence)
        .map(|values| {
            values
                .iter()
                .filter_map(YamlValue::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let lifecycle_status = string_field(mapping, "status")
        .map(str::trim)
        .filter(|status| !status.is_empty())
        .unwrap_or("stable")
        .to_owned();
    let declared_generated = mapping
        .get(YamlValue::String("generated".to_owned()))
        .cloned();
    let legacy_timestamp = string_field(mapping, "timestamp")
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc));
    let generated = declared_generated.clone().or_else(|| {
        legacy_timestamp.map(|at| {
            YamlValue::Mapping(
                [(
                    YamlValue::String("at".to_owned()),
                    YamlValue::String(at.to_rfc3339()),
                )]
                .into_iter()
                .collect(),
            )
        })
    });
    let verified = mapping
        .get(YamlValue::String("verified".to_owned()))
        .cloned();
    let declared_sources = mapping
        .get(YamlValue::String("sources".to_owned()))
        .cloned();
    let sources = declared_sources
        .clone()
        .or_else(|| legacy_citation_sources(parsed.body));
    let stale_after = string_field(mapping, "stale_after").map(ToOwned::to_owned);
    let mut warnings = optional_metadata_warnings(mapping, path);
    if declared_generated.is_none() && legacy_timestamp.is_some() {
        warnings.push(warning(
            OkfWarningCode::LegacyTimestamp,
            path,
            Some("timestamp"),
        ));
    }
    if declared_sources.is_none() && sources.is_some() {
        warnings.push(warning(
            OkfWarningCode::LegacyCitations,
            path,
            Some("# Citations"),
        ));
    }
    let parsed_generated = valid_generated(declared_generated.as_ref());
    let generated_at = if declared_generated.is_some() {
        parsed_generated.as_ref().and_then(|(_, at)| *at)
    } else {
        legacy_timestamp
    };
    if declared_generated.is_some() && parsed_generated.is_none() {
        warnings.push(warning(
            OkfWarningCode::InvalidGenerated,
            path,
            Some("generated"),
        ));
    }
    let valid_verifications = valid_verifications(verified.as_ref());
    if verified
        .as_ref()
        .is_some_and(|value| !verifications_are_well_formed(value))
    {
        warnings.push(warning(
            OkfWarningCode::InvalidVerified,
            path,
            Some("verified"),
        ));
    }
    if declared_sources
        .as_ref()
        .is_some_and(|value| !valid_sources(value) || !sources_have_valid_optional_metadata(value))
    {
        warnings.push(warning(
            OkfWarningCode::InvalidSources,
            path,
            Some("sources"),
        ));
    }
    if mapping
        .get(YamlValue::String("usage_window".to_owned()))
        .is_some_and(|value| !valid_usage_window(value))
    {
        warnings.push(warning(
            OkfWarningCode::InvalidOptionalMetadata,
            path,
            Some("usage_window"),
        ));
    }
    let freshness = match stale_after.as_deref() {
        None => FreshnessState::NotDeclared,
        Some(value) => match NaiveDate::parse_from_str(value, "%Y-%m-%d") {
            Ok(date) if now.date_naive() >= date => FreshnessState::Stale,
            Ok(_) => FreshnessState::Fresh,
            Err(_) => {
                warnings.push(warning(
                    OkfWarningCode::InvalidStaleAfter,
                    path,
                    Some("stale_after"),
                ));
                FreshnessState::Invalid
            }
        },
    };
    let trust = if valid_verifications
        .iter()
        .any(|(actor, _)| actor.is_human())
    {
        TrustTier::HumanReviewed
    } else if valid_verifications.is_empty() {
        TrustTier::Unverified
    } else {
        TrustTier::MachineConfirmed
    };
    let verification_outdated = !valid_verifications.is_empty()
        && generated_at.is_some_and(|generated_at| {
            valid_verifications
                .iter()
                .map(|(_, at)| *at)
                .max()
                .is_none_or(|verified_at| verified_at < generated_at)
        });
    let attested_computation = if concept_type == Some("Attested Computation") {
        match parse_attested_computation(mapping) {
            Some(contract) if contract.runtime == "airwiki-wasm" => Some(contract),
            Some(_) => {
                warnings.push(warning(
                    OkfWarningCode::UnsupportedRuntime,
                    path,
                    Some("runtime"),
                ));
                None
            }
            None => {
                warnings.push(warning(
                    OkfWarningCode::InvalidAttestedComputation,
                    path,
                    Some("runtime"),
                ));
                None
            }
        }
    } else {
        None
    };
    let known = [
        "type",
        "title",
        "description",
        "resource",
        "tags",
        "status",
        "generated",
        "verified",
        "sources",
        "usage_window",
        "stale_after",
        "version",
        "timestamp",
    ];
    let unknown_frontmatter = YamlValue::Mapping(
        mapping
            .iter()
            .filter(|(key, _)| key.as_str().is_none_or(|key| !known.contains(&key)))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    );
    Ok(OkfImportedConcept {
        logical_path: path.to_owned(),
        concept_type: concept_type.unwrap_or_default().to_owned(),
        title,
        description,
        tags,
        lifecycle_status,
        generated,
        verified: verified.map(normalize_verifications),
        sources,
        stale_after,
        version: string_field(mapping, "version").map(ToOwned::to_owned),
        unknown_frontmatter,
        attested_computation,
        fingerprint: hex::encode(sha2::Sha256::digest(markdown.as_bytes())),
        search_text: parsed.body.to_owned(),
        assurance: ConceptAssurance {
            trust,
            freshness,
            verification_outdated,
        },
        warnings,
    })
}

fn concept_has_v02_signal(concept: &OkfImportedConcept) -> bool {
    concept
        .generated
        .as_ref()
        .and_then(|value| valid_generated(Some(value)))
        .is_some()
        || concept.verified.is_some()
        || (concept.sources.is_some()
            && !concept
                .warnings
                .iter()
                .any(|warning| warning.code == OkfWarningCode::LegacyCitations))
        || concept.stale_after.is_some()
        || concept.lifecycle_status != "stable"
        || concept.attested_computation.is_some()
}

fn parse_attested_computation(
    mapping: &serde_yaml::Mapping,
) -> Option<AttestedComputationContract> {
    let runtime = string_field(mapping, "runtime")?.trim();
    if runtime.is_empty() {
        return None;
    }
    let parameter_values: &[YamlValue] =
        match mapping.get(YamlValue::String("parameters".to_owned())) {
            Some(value) => value.as_sequence()?.as_slice(),
            None => &[],
        };
    let parameters = parameter_values
        .iter()
        .map(parse_attested_parameter)
        .collect::<Option<Vec<_>>>()?;
    let executor = parse_executor(mapping.get(YamlValue::String("executor".to_owned()))?)?;
    let attester = parse_artifact(mapping.get(YamlValue::String("attester".to_owned()))?)?;
    Some(AttestedComputationContract {
        runtime: runtime.to_owned(),
        parameters,
        computation: string_field(mapping, "computation").map(ToOwned::to_owned),
        executor,
        attester,
    })
}

fn parse_attested_parameter(value: &YamlValue) -> Option<AttestedParameter> {
    let mapping = value.as_mapping()?;
    let name = string_field(mapping, "name")?.trim();
    let parameter_type = string_field(mapping, "type")?.trim();
    let required = mapping
        .get(YamlValue::String("required".to_owned()))
        .and_then(YamlValue::as_bool)
        .unwrap_or(false);
    if name.is_empty() || parameter_type.is_empty() {
        return None;
    }
    Some(AttestedParameter {
        name: name.to_owned(),
        parameter_type: parameter_type.to_owned(),
        required,
    })
}

fn parse_executor(value: &YamlValue) -> Option<AttestedExecutor> {
    let mapping = value.as_mapping()?;
    let artifact = parse_artifact(value)?;
    let receipt_values: &[YamlValue] = match mapping.get(YamlValue::String("receipt".to_owned())) {
        Some(value) => value.as_sequence()?.as_slice(),
        None => &[],
    };
    let receipt = receipt_values
        .iter()
        .map(YamlValue::as_str)
        .map(|value| value.map(ToOwned::to_owned))
        .collect::<Option<Vec<_>>>()?;
    Some(AttestedExecutor {
        resource: artifact.resource,
        sha256: artifact.sha256,
        receipt,
    })
}

fn parse_artifact(value: &YamlValue) -> Option<AttestedArtifact> {
    let mapping = value.as_mapping()?;
    let resource = string_field(mapping, "resource")?.trim();
    let sha256 = string_field(mapping, "sha256")?.trim();
    if resource.is_empty()
        || sha256.len() != 64
        || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }
    Some(AttestedArtifact {
        resource: resource.to_owned(),
        sha256: sha256.to_ascii_lowercase(),
    })
}

#[cfg(test)]
mod attested_tests {
    use super::*;

    #[test]
    fn airwiki_wasm_contract_is_normalized_without_losing_extension_fields() {
        let markdown = r#"---
type: Attested Computation
title: Revenue
runtime: airwiki-wasm
parameters:
  - { name: year, type: integer, required: true }
executor:
  resource: references/executor.wasm
  sha256: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
  receipt: [result]
attester:
  resource: references/attester.wasm
  sha256: bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
vendor_extension: preserved
---
# Computation
"#;

        let concept = parse_concept("computations/revenue.md", markdown, Utc::now()).unwrap();
        let contract = concept.attested_computation.unwrap();

        assert_eq!(contract.runtime, "airwiki-wasm");
        assert_eq!(contract.parameters[0].name, "year");
        assert_eq!(contract.executor.receipt, ["result"]);
        assert!(
            concept
                .unknown_frontmatter
                .as_mapping()
                .unwrap()
                .contains_key(YamlValue::String("vendor_extension".to_owned()))
        );
    }

    #[test]
    fn unsupported_or_incomplete_computation_is_preserved_but_not_executable() {
        let markdown = r#"---
type: Attested Computation
runtime: python
executor: { resource: run.py }
attester: { resource: attest.py }
---
"#;

        let concept = parse_concept("computation.md", markdown, Utc::now()).unwrap();

        assert!(concept.attested_computation.is_none());
        assert!(
            concept
                .warnings
                .iter()
                .any(|warning| warning.code == OkfWarningCode::InvalidAttestedComputation)
        );
        assert!(
            concept
                .unknown_frontmatter
                .as_mapping()
                .unwrap()
                .contains_key(YamlValue::String("runtime".to_owned()))
        );
    }

    #[test]
    fn malformed_optional_source_signals_warn_without_rejecting_the_concept() {
        let markdown = r#"---
type: Reference
sources:
  - resource: docs/source.md
    id: 42
    title: [not, text]
    author: { unexpected: mapping }
    usage_count: -1
    last_modified: false
    usage_window: { from: 2026-08-03, to: 2026-08-01 }
---
"#;

        let concept = parse_concept("reference.md", markdown, Utc::now()).unwrap();

        assert_eq!(concept.assurance.trust, TrustTier::Unverified);
        assert!(concept.sources.is_some());
        assert!(concept.warnings.iter().any(|warning| {
            warning.code == OkfWarningCode::InvalidSources
                && warning.field.as_deref() == Some("sources")
        }));
    }
}

fn string_field<'a>(mapping: &'a serde_yaml::Mapping, name: &str) -> Option<&'a str> {
    mapping
        .get(YamlValue::String(name.to_owned()))
        .and_then(YamlValue::as_str)
}

fn valid_generated(value: Option<&YamlValue>) -> Option<(ActorId, Option<DateTime<Utc>>)> {
    let mapping = value?.as_mapping()?;
    let actor = string_field(mapping, "by")?.parse().ok()?;
    let at = match mapping.get(YamlValue::String("at".to_owned())) {
        None => None,
        Some(YamlValue::String(value)) => Some(
            DateTime::parse_from_rfc3339(value)
                .ok()?
                .with_timezone(&Utc),
        ),
        Some(_) => return None,
    };
    Some((actor, at))
}

fn valid_verifications(value: Option<&YamlValue>) -> Vec<(ActorId, DateTime<Utc>)> {
    let values: Vec<&YamlValue> = match value {
        Some(YamlValue::Sequence(values)) => values.iter().collect(),
        Some(value @ YamlValue::Mapping(_)) => vec![value],
        _ => Vec::new(),
    };
    values
        .into_iter()
        .filter_map(|value| {
            let mapping = value.as_mapping()?;
            let actor = string_field(mapping, "by")?.parse().ok()?;
            let at = DateTime::parse_from_rfc3339(string_field(mapping, "at")?)
                .ok()?
                .with_timezone(&Utc);
            Some((actor, at))
        })
        .collect()
}

fn verifications_are_well_formed(value: &YamlValue) -> bool {
    let values: Vec<&YamlValue> = match value {
        YamlValue::Sequence(values) if !values.is_empty() => values.iter().collect(),
        value @ YamlValue::Mapping(_) => vec![value],
        _ => return false,
    };
    values.into_iter().all(|value| {
        let Some(mapping) = value.as_mapping() else {
            return false;
        };
        string_field(mapping, "by")
            .and_then(|actor| actor.parse::<ActorId>().ok())
            .is_some()
            && string_field(mapping, "at")
                .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
                .is_some()
    })
}

fn valid_sources(value: &YamlValue) -> bool {
    value.as_sequence().is_some_and(|sources| {
        sources.iter().all(|source| {
            source.as_mapping().is_some_and(|mapping| {
                string_field(mapping, "resource")
                    .is_some_and(|resource| !resource.trim().is_empty())
            })
        })
    })
}

fn sources_have_valid_optional_metadata(value: &YamlValue) -> bool {
    value.as_sequence().is_some_and(|sources| {
        sources.iter().all(|source| {
            let Some(mapping) = source.as_mapping() else {
                return false;
            };
            let id_valid =
                optional_string_field_is_valid(mapping, "id", |id| !id.trim().is_empty());
            let title_valid =
                optional_string_field_is_valid(mapping, "title", |title| !title.trim().is_empty());
            let author_valid = optional_string_field_is_valid(mapping, "author", |author| {
                author.parse::<ActorId>().is_ok()
            });
            let usage_valid = mapping
                .get(YamlValue::String("usage_count".to_owned()))
                .map(|usage| usage.as_u64().is_some())
                .unwrap_or(true);
            let modified_valid = optional_string_field_is_valid(mapping, "last_modified", |date| {
                NaiveDate::parse_from_str(date, "%Y-%m-%d").is_ok()
            });
            let window_valid = mapping
                .get(YamlValue::String("usage_window".to_owned()))
                .map(valid_usage_window)
                .unwrap_or(true);
            id_valid && title_valid && author_valid && usage_valid && modified_valid && window_valid
        })
    })
}

fn optional_string_field_is_valid(
    mapping: &serde_yaml::Mapping,
    name: &str,
    validate: impl FnOnce(&str) -> bool,
) -> bool {
    match mapping.get(YamlValue::String(name.to_owned())) {
        None => true,
        Some(YamlValue::String(value)) => validate(value),
        Some(_) => false,
    }
}

fn valid_usage_window(value: &YamlValue) -> bool {
    let Some(mapping) = value.as_mapping() else {
        return false;
    };
    let Some(from) = string_field(mapping, "from")
        .and_then(|date| NaiveDate::parse_from_str(date, "%Y-%m-%d").ok())
    else {
        return false;
    };
    let Some(to) = string_field(mapping, "to")
        .and_then(|date| NaiveDate::parse_from_str(date, "%Y-%m-%d").ok())
    else {
        return false;
    };
    from <= to
}

fn optional_metadata_warnings(mapping: &serde_yaml::Mapping, path: &str) -> Vec<OkfWarning> {
    let scalar_fields = ["title", "description", "resource", "status", "version"];
    let mut warnings = scalar_fields
        .into_iter()
        .filter(|field| {
            mapping
                .get(YamlValue::String((*field).to_owned()))
                .is_some_and(|value| value.as_str().is_none())
        })
        .map(|field| warning(OkfWarningCode::InvalidOptionalMetadata, path, Some(field)))
        .collect::<Vec<_>>();
    if mapping
        .get(YamlValue::String("tags".to_owned()))
        .is_some_and(|value| {
            value
                .as_sequence()
                .is_none_or(|tags| tags.iter().any(|tag| tag.as_str().is_none()))
        })
    {
        warnings.push(warning(
            OkfWarningCode::InvalidOptionalMetadata,
            path,
            Some("tags"),
        ));
    }
    warnings
}

fn legacy_citation_sources(body: &str) -> Option<YamlValue> {
    let mut in_citations = false;
    let mut resources = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            if trimmed == "# Citations" {
                in_citations = true;
                continue;
            }
            if in_citations {
                break;
            }
        }
        if !in_citations {
            continue;
        }
        let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        else {
            continue;
        };
        let resource = markdown_links(item)
            .into_iter()
            .next()
            .unwrap_or_else(|| item.trim().to_owned());
        if resource.is_empty() || resource.len() > 2_048 || resources.len() >= 256 {
            continue;
        }
        resources.push(YamlValue::Mapping(
            [(
                YamlValue::String("resource".to_owned()),
                YamlValue::String(resource),
            )]
            .into_iter()
            .collect(),
        ));
    }
    (!resources.is_empty()).then_some(YamlValue::Sequence(resources))
}

fn validate_log(markdown: &str) -> Result<()> {
    if split_frontmatter(markdown)?.yaml.is_some() {
        bail!("OKF log.md must not contain frontmatter");
    }
    let mut lines = markdown.lines();
    if !lines.next().is_some_and(|line| line.starts_with("# ")) {
        bail!("OKF log.md must start with an H1 heading");
    }
    let mut previous = None;
    let mut current = None;
    for line in lines.map(str::trim).filter(|line| !line.is_empty()) {
        if let Some(value) = line.strip_prefix("## ") {
            let date = NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d")
                .context("OKF log entries must use YYYY-MM-DD headings")?;
            if previous.is_some_and(|previous| date >= previous) {
                bail!("OKF log entries must be unique and newest first");
            }
            previous = Some(date);
            current = Some(date);
        } else if !(line.starts_with("* ") || line.starts_with("- ")) || current.is_none() {
            bail!("OKF log entries must be list items below a date heading");
        }
    }
    Ok(())
}

fn validate_index_body(markdown: &str) -> Result<()> {
    let mut lines = markdown
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    if !lines.next().is_some_and(|line| line.starts_with("# ")) {
        bail!("OKF index.md must start with an H1 section heading");
    }
    for line in lines {
        if line.starts_with("# ") || line.starts_with("* [") || line.starts_with("- [") {
            continue;
        }
        bail!("OKF index.md contains content outside a section or link list");
    }
    Ok(())
}

fn validated_relative_path(path: &Path) -> Result<String> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        bail!("OKF path must be relative");
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => {
                let value = value.to_str().context("OKF paths must be UTF-8")?;
                if value.is_empty() || value.contains(['\\', ':']) {
                    bail!("OKF path contains a forbidden component");
                }
                parts.push(value);
            }
            _ => bail!("OKF path contains traversal"),
        }
    }
    Ok(parts.join("/"))
}

fn portable_path_key(path: &str) -> String {
    path.nfc().flat_map(char::to_lowercase).collect()
}

fn file_limit(path: &str) -> u64 {
    if path.to_ascii_lowercase().ends_with(".md") {
        MAX_CONCEPT_BYTES
    } else {
        MAX_RESOURCE_BYTES
    }
}

fn normalize_verifications(value: YamlValue) -> YamlValue {
    match value {
        YamlValue::Mapping(_) => YamlValue::Sequence(vec![value]),
        value => value,
    }
}

fn yaml_depth(value: &YamlValue, depth: usize) -> usize {
    match value {
        YamlValue::Sequence(values) => values
            .iter()
            .map(|value| yaml_depth(value, depth + 1))
            .max()
            .unwrap_or(depth),
        YamlValue::Mapping(values) => values
            .iter()
            .map(|(key, value)| yaml_depth(key, depth + 1).max(yaml_depth(value, depth + 1)))
            .max()
            .unwrap_or(depth),
        YamlValue::Tagged(value) => yaml_depth(&value.value, depth + 1),
        _ => depth,
    }
}

fn markdown_links(markdown: &str) -> Vec<String> {
    pulldown_cmark::Parser::new(markdown)
        .filter_map(|event| match event {
            pulldown_cmark::Event::Start(pulldown_cmark::Tag::Link { dest_url, .. }) => {
                Some(dest_url.into_string())
            }
            _ => None,
        })
        .collect()
}

fn is_relative_markdown_target(target: &str) -> bool {
    !target.starts_with(['/', '#'])
        && !target.contains("://")
        && target
            .split(['#', '?'])
            .next()
            .is_some_and(|path| path.to_ascii_lowercase().ends_with(".md"))
}

fn resolve_relative_target(source: &str, target: &str) -> Option<String> {
    let target = target.split(['#', '?']).next()?;
    let mut base = PathBuf::from(source);
    base.pop();
    base.push(target);
    validated_relative_path(&base).ok()
}

fn warning(
    code: OkfWarningCode,
    logical_path: impl Into<String>,
    field: Option<&str>,
) -> OkfWarning {
    OkfWarning {
        code,
        logical_path: logical_path.into(),
        field: field.map(ToOwned::to_owned),
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use super::*;

    const CONCEPT: &[u8] =
        b"---\ntype: Future Knowledge\nstatus: custom\nx-extension: true\n---\n\n# Item\n";

    #[test]
    fn accepts_minimal_bundle_without_index() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("concept.md"), CONCEPT).unwrap();
        let report = OkfImportValidator::validate_directory(temp.path()).unwrap();
        assert_eq!(
            report.compatibility,
            OkfCompatibility::UndeclaredV02Compatible
        );
    }

    #[test]
    fn accepts_crlf_frontmatter_with_exact_delimiter() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("concept.md"),
            b"---\r\ntype: Reference\r\n---\r\nBody",
        )
        .unwrap();
        assert!(OkfImportValidator::validate_directory(temp.path()).is_ok());
    }

    #[test]
    fn accepts_frontmatter_closing_delimiter_at_end_of_file() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("concept.md"), b"---\ntype: Reference\n---").unwrap();

        assert!(OkfImportValidator::validate_directory(temp.path()).is_ok());
    }

    #[test]
    fn rejects_nested_index_frontmatter() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("nested")).unwrap();
        fs::write(
            temp.path().join("nested/index.md"),
            b"---\ntype: Reference\n---\n",
        )
        .unwrap();
        assert!(OkfImportValidator::validate_directory(temp.path()).is_err());
    }

    #[test]
    fn malformed_verification_never_increases_trust() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("concept.md"),
            b"---\ntype: Reference\nverified:\n  by: human:owner\n---\n",
        )
        .unwrap();
        let report = OkfImportValidator::validate_directory(temp.path()).unwrap();
        assert_eq!(report.concepts[0].assurance.trust, TrustTier::Unverified);
    }

    #[test]
    fn generated_by_without_optional_timestamp_is_valid_and_not_outdated() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("concept.md"),
            b"---\ntype: Reference\ngenerated: { by: producer/1 }\n---\n",
        )
        .unwrap();

        let report = OkfImportValidator::validate_directory(temp.path()).unwrap();
        let concept = report.concepts.first().unwrap();

        assert!(concept.warnings.is_empty());
        assert!(!concept.assurance.verification_outdated);
    }

    #[test]
    fn generated_content_without_verification_is_unverified_not_outdated() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("concept.md"),
            b"---\ntype: Reference\ngenerated: { by: producer/1, at: 2026-08-01T00:00:00Z }\n---\n",
        )
        .unwrap();

        let report = OkfImportValidator::validate_directory(temp.path()).unwrap();
        let assurance = report.concepts.first().unwrap().assurance;

        assert_eq!(assurance.trust, TrustTier::Unverified);
        assert!(!assurance.verification_outdated);
    }

    #[test]
    fn mixed_valid_and_invalid_verification_is_warned_but_keeps_valid_trust() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("concept.md"),
            b"---\ntype: Reference\nverified:\n  - { by: human:reviewer, at: 2026-08-02T00:00:00Z }\n  - { by: invalid actor, at: later }\n---\n",
        )
        .unwrap();

        let report = OkfImportValidator::validate_directory(temp.path()).unwrap();
        let concept = report.concepts.first().unwrap();

        assert_eq!(concept.assurance.trust, TrustTier::HumanReviewed);
        assert!(
            concept
                .warnings
                .iter()
                .any(|warning| warning.code == OkfWarningCode::InvalidVerified)
        );
    }

    #[test]
    fn invalid_optional_source_signals_are_preserved_and_warned() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("concept.md"),
            b"---\ntype: Reference\nsources:\n  - resource: source.md\n    author: invalid actor\n    usage_count: many\n    last_modified: yesterday\nusage_window: { from: 2026-08-02, to: 2026-08-01 }\n---\n",
        )
        .unwrap();

        let report = OkfImportValidator::validate_directory(temp.path()).unwrap();
        let concept = report.concepts.first().unwrap();

        assert!(concept.sources.is_some());
        assert!(
            concept
                .warnings
                .iter()
                .any(|warning| warning.code == OkfWarningCode::InvalidSources)
        );
        assert!(
            concept
                .warnings
                .iter()
                .any(|warning| warning.field.as_deref() == Some("usage_window"))
        );
    }

    #[test]
    fn reserved_indexes_and_logs_must_follow_their_structure() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("index.md"), b"free prose\n").unwrap();
        assert!(OkfImportValidator::validate_directory(temp.path()).is_err());

        fs::write(
            temp.path().join("index.md"),
            b"# Root\n\n- [Concept](concept.md)\n",
        )
        .unwrap();
        fs::write(
            temp.path().join("log.md"),
            b"# Log\n\n## 2026-08-01\nprose outside a list\n",
        )
        .unwrap();
        assert!(OkfImportValidator::validate_directory(temp.path()).is_err());
    }

    #[test]
    fn legacy_timestamp_and_citations_are_normalized_without_inventing_an_actor() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("legacy.md"),
            "---\ntype: Metric\ntimestamp: '2026-05-28T22:53:05+00:00'\n---\n\n# Definition\n\nValue.\n\n# Citations\n\n- https://example.invalid/policy\n",
        )
        .unwrap();

        let report = OkfImportValidator::validate_directory(temp.path()).unwrap();
        let concept = report.concepts.first().unwrap();
        let generated = concept.generated.as_ref().unwrap().as_mapping().unwrap();
        assert!(generated.get(YamlValue::String("by".to_owned())).is_none());
        assert_eq!(
            generated
                .get(YamlValue::String("at".to_owned()))
                .and_then(YamlValue::as_str),
            Some("2026-05-28T22:53:05+00:00")
        );
        assert!(valid_sources(concept.sources.as_ref().unwrap()));
        assert_eq!(concept.assurance.trust, TrustTier::Unverified);
        assert!(
            concept
                .warnings
                .iter()
                .any(|warning| warning.code == OkfWarningCode::LegacyTimestamp)
        );
        assert!(
            concept
                .warnings
                .iter()
                .any(|warning| warning.code == OkfWarningCode::LegacyCitations)
        );
        assert_eq!(report.compatibility, OkfCompatibility::LegacyV01);
    }

    #[test]
    fn undeclared_bundle_with_v02_metadata_is_not_misclassified_as_legacy() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("concept.md"),
            b"---\ntype: Reference\ngenerated: { by: producer/1 }\n---\n",
        )
        .unwrap();

        let report = OkfImportValidator::validate_directory(temp.path()).unwrap();

        assert_eq!(
            report.compatibility,
            OkfCompatibility::UndeclaredV02Compatible
        );
    }

    #[test]
    fn future_version_is_imported_restricted() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("index.md"),
            b"---\nokf_version: '0.3'\n---\n# Future\n",
        )
        .unwrap();
        fs::write(temp.path().join("concept.md"), CONCEPT).unwrap();
        let report = OkfImportValidator::validate_directory(temp.path()).unwrap();
        assert!(matches!(
            report.compatibility,
            OkfCompatibility::FutureRestricted { .. }
        ));
    }

    #[test]
    fn zip_rejects_case_insensitive_collisions() {
        let cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        writer
            .start_file("A.md", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(CONCEPT).unwrap();
        writer
            .start_file("a.md", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(CONCEPT).unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        assert!(OkfImportValidator::validate_zip(Cursor::new(bytes)).is_err());
    }

    #[test]
    fn zip_rejects_file_directory_prefix_collisions() {
        let cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        writer
            .start_file("folder", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"file").unwrap();
        writer
            .start_file(
                "folder/concept.md",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer.write_all(CONCEPT).unwrap();
        let bytes = writer.finish().unwrap().into_inner();

        assert!(OkfImportValidator::validate_zip(Cursor::new(bytes)).is_err());
    }

    #[test]
    fn explicit_zip_directories_count_toward_the_entry_budget() {
        let cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        for index in 0..=MAX_ENTRIES {
            writer
                .add_directory(
                    format!("directory-{index}/"),
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
        }
        let bytes = writer.finish().unwrap().into_inner();

        assert!(OkfImportValidator::validate_zip(Cursor::new(bytes)).is_err());
    }

    #[test]
    fn materialization_preserves_source_bytes() {
        let source = tempfile::tempdir().unwrap();
        let parent = tempfile::tempdir().unwrap();
        let staging = parent.path().join("staging");
        fs::write(source.path().join("concept.md"), CONCEPT).unwrap();
        OkfImportValidator::materialize_path(source.path(), &staging).unwrap();
        assert_eq!(fs::read(staging.join("concept.md")).unwrap(), CONCEPT);
    }
}
