use std::collections::{BTreeSet, HashSet};
use std::fs::{self, File};
use std::io::{Read, Seek};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_yaml::Value as YamlValue;
use sha2::Digest;
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
pub struct OkfImportWarning {
    pub code: &'static str,
    pub logical_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OkfImportReport {
    pub entry_count: usize,
    pub concept_count: usize,
    pub uncompressed_bytes: u64,
    pub okf_version: String,
    pub warnings: Vec<OkfImportWarning>,
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
    pub version: Option<String>,
    pub unknown_frontmatter: YamlValue,
    pub fingerprint: String,
    pub search_text: String,
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
            let limit = file_limit(&logical_path);
            if metadata.len() > limit {
                bail!("OKF import entry exceeds its size limit");
            }
            let bytes = fs::read(entry.path()).context("could not read OKF import entry")?;
            state.accept(logical_path, &bytes)?;
        }
        state.finish()
    }

    pub fn validate_zip<R: Read + Seek>(reader: R) -> Result<OkfImportReport> {
        let mut archive = ZipArchive::new(reader).context("invalid OKF ZIP archive")?;
        let mut state = ValidationState::default();
        let mut names = HashSet::new();
        for index in 0..archive.len() {
            let entry = archive
                .by_index(index)
                .context("could not inspect ZIP entry")?;
            let Some(enclosed) = entry.enclosed_name() else {
                bail!("ZIP entry contains an absolute path or traversal");
            };
            let logical_path = validated_relative_path(&enclosed)?;
            if !names.insert(logical_path.clone()) {
                bail!("ZIP contains duplicate entries");
            }
            if entry.is_dir() {
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
            let capacity = usize::try_from(entry.size()).context("ZIP entry is too large")?;
            let mut bytes = Vec::with_capacity(capacity);
            entry
                .take(file_limit(&logical_path).saturating_add(1))
                .read_to_end(&mut bytes)
                .context("could not read ZIP entry")?;
            state.accept(logical_path, &bytes)?;
        }
        state.finish()
    }

    /// Copies a validated bundle into a new managed staging directory and
    /// validates the copied bytes again before returning.
    pub fn materialize_path(source: &Path, staging: &Path) -> Result<OkfImportReport> {
        if staging.exists() {
            bail!("OKF import staging directory already exists");
        }
        let expected = Self::validate_path(source)?;
        fs::create_dir(staging).context("could not create OKF import staging directory")?;
        let materialized = (|| {
            let metadata = fs::symlink_metadata(source)?;
            if metadata.is_dir() {
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

fn copy_directory(source: &Path, staging: &Path) -> Result<()> {
    for entry in WalkDir::new(source).follow_links(false) {
        let entry = entry.context("OKF directory traversal failed while copying")?;
        if entry.path() == source {
            continue;
        }
        let relative = entry.path().strip_prefix(source)?;
        let logical_path = validated_relative_path(relative)?;
        let destination = staging.join(&logical_path);
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata_is_link_or_reparse(&metadata) {
            bail!("OKF import changed to a link while being copied");
        }
        if metadata.is_dir() {
            fs::create_dir(&destination).context("could not create staged OKF directory")?;
        } else if metadata.is_file() {
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(entry.path(), &destination).context("could not copy staged OKF entry")?;
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
        if !names.insert(logical_path.clone()) {
            bail!("ZIP contains duplicate entries");
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
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = File::create(&destination)?;
        std::io::copy(&mut entry, &mut output)?;
    }
    Ok(())
}

fn metadata_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        return metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    }
    #[cfg(not(windows))]
    false
}

#[derive(Default)]
struct ValidationState {
    entries: usize,
    concepts: usize,
    bytes: u64,
    root_version: Option<String>,
    paths: BTreeSet<String>,
    links: Vec<(String, String)>,
    concepts_metadata: Vec<OkfImportedConcept>,
}

impl ValidationState {
    fn accept(&mut self, path: String, bytes: &[u8]) -> Result<()> {
        self.entries = self.entries.saturating_add(1);
        if self.entries > MAX_ENTRIES {
            bail!("OKF import contains too many entries");
        }
        self.bytes = self
            .bytes
            .checked_add(u64::try_from(bytes.len())?)
            .context("OKF import size overflow")?;
        if self.bytes > MAX_TOTAL_BYTES {
            bail!("OKF import exceeds the uncompressed size limit");
        }
        if !self.paths.insert(path.clone()) {
            bail!("OKF import contains duplicate paths");
        }
        if path.ends_with(".md") {
            let markdown = std::str::from_utf8(bytes).context("OKF Markdown must be UTF-8")?;
            let filename = path.rsplit('/').next().unwrap_or(path.as_str());
            if filename.eq_ignore_ascii_case("index.md") {
                if !path.contains('/') {
                    self.root_version = root_okf_version(markdown)?;
                }
            } else if !filename.eq_ignore_ascii_case("log.md") {
                self.concepts_metadata.push(parse_concept(&path, markdown)?);
                self.concepts = self.concepts.saturating_add(1);
            }
            self.links.extend(
                markdown_links(markdown)
                    .into_iter()
                    .map(|target| (path.clone(), target)),
            );
        }
        Ok(())
    }

    fn finish(self) -> Result<OkfImportReport> {
        let version = self
            .root_version
            .context("root index.md must declare okf_version 0.2")?;
        if version != "0.2" {
            bail!("only OKF v0.2 bundles can be imported");
        }
        let warnings = self
            .links
            .into_iter()
            .filter(|(_, target)| is_relative_markdown_target(target))
            .filter_map(|(source, target)| {
                resolve_relative_target(&source, &target)
                    .filter(|resolved| !self.paths.contains(resolved))
                    .map(|_| OkfImportWarning {
                        code: "broken_link",
                        logical_path: source,
                    })
            })
            .collect();
        Ok(OkfImportReport {
            entry_count: self.entries,
            concept_count: self.concepts,
            uncompressed_bytes: self.bytes,
            okf_version: version,
            warnings,
            concepts: self.concepts_metadata,
        })
    }
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

fn file_limit(path: &str) -> u64 {
    if path.ends_with(".md") {
        MAX_CONCEPT_BYTES
    } else {
        MAX_RESOURCE_BYTES
    }
}

fn frontmatter(markdown: &str) -> Result<Option<YamlValue>> {
    let Some(rest) = markdown.strip_prefix("---\n") else {
        return Ok(None);
    };
    let end = rest
        .find("\n---")
        .context("frontmatter is not terminated")?;
    if end > MAX_FRONTMATTER_BYTES {
        bail!("frontmatter exceeds 64 KiB")
    }
    let yaml: YamlValue = serde_yaml::from_str(&rest[..end]).context("invalid YAML frontmatter")?;
    if yaml_depth(&yaml, 0) > MAX_YAML_DEPTH {
        bail!("YAML nesting is too deep")
    }
    Ok(Some(yaml))
}

fn parse_concept(path: &str, markdown: &str) -> Result<OkfImportedConcept> {
    let yaml = frontmatter(markdown)?.context("OKF concept is missing frontmatter")?;
    let mapping = yaml
        .as_mapping()
        .context("OKF frontmatter must be a mapping")?;
    let concept_type = mapping
        .get(YamlValue::String("type".to_owned()))
        .and_then(YamlValue::as_str)
        .map(str::trim);
    if concept_type.is_none_or(str::is_empty) {
        bail!("OKF concept type is required")
    }
    let title = mapping
        .get(YamlValue::String("title".to_owned()))
        .and_then(YamlValue::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            path.rsplit('/')
                .next()
                .unwrap_or(path)
                .trim_end_matches(".md")
                .to_owned()
        });
    let description = mapping
        .get(YamlValue::String("description".to_owned()))
        .and_then(YamlValue::as_str)
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
    let lifecycle_status = mapping
        .get(YamlValue::String("status".to_owned()))
        .and_then(YamlValue::as_str)
        .unwrap_or("stable");
    if !matches!(lifecycle_status, "draft" | "stable" | "deprecated") {
        bail!("OKF lifecycle status is invalid");
    }
    let known = [
        "type",
        "title",
        "description",
        "tags",
        "status",
        "generated",
        "verified",
        "sources",
        "version",
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
        lifecycle_status: lifecycle_status.to_owned(),
        generated: mapping
            .get(YamlValue::String("generated".to_owned()))
            .cloned(),
        verified: mapping
            .get(YamlValue::String("verified".to_owned()))
            .cloned()
            .map(normalize_verifications),
        sources: mapping
            .get(YamlValue::String("sources".to_owned()))
            .cloned(),
        version: mapping
            .get(YamlValue::String("version".to_owned()))
            .and_then(YamlValue::as_str)
            .map(ToOwned::to_owned),
        unknown_frontmatter,
        fingerprint: hex::encode(sha2::Sha256::digest(markdown.as_bytes())),
        search_text: markdown_body(markdown).unwrap_or_default().to_owned(),
    })
}

fn markdown_body(markdown: &str) -> Option<&str> {
    let rest = markdown.strip_prefix("---\n")?;
    let end = rest.find("\n---\n")?;
    Some(&rest[end + 5..])
}

fn normalize_verifications(value: YamlValue) -> YamlValue {
    match value {
        YamlValue::Mapping(_) => YamlValue::Sequence(vec![value]),
        value => value,
    }
}

fn root_okf_version(markdown: &str) -> Result<Option<String>> {
    Ok(frontmatter(markdown)?.and_then(|yaml| {
        yaml.as_mapping()
            .and_then(|mapping| mapping.get(YamlValue::String("okf_version".to_owned())))
            .and_then(YamlValue::as_str)
            .map(ToOwned::to_owned)
    }))
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
            .is_some_and(|path| path.ends_with(".md"))
}

fn resolve_relative_target(source: &str, target: &str) -> Option<String> {
    let target = target.split(['#', '?']).next()?;
    let mut base = PathBuf::from(source);
    base.pop();
    base.push(target);
    validated_relative_path(&base).ok()
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use super::*;

    fn valid_index() -> &'static [u8] {
        b"---\nokf_version: \"0.2\"\n---\n\n# Knowledge\n"
    }

    fn valid_concept() -> &'static [u8] {
        b"---\ntype: Future Knowledge\nstatus: stable\nx-extension: true\n---\n\n# Item\n"
    }

    #[test]
    fn directory_accepts_unknown_types_and_reports_broken_links() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("index.md"), valid_index()).unwrap();
        fs::create_dir(temp.path().join("nested")).unwrap();
        fs::write(
            temp.path().join("nested/item.md"),
            [valid_concept(), b"\n[Missing](missing.md)"].concat(),
        )
        .unwrap();

        let report = OkfImportValidator::validate_directory(temp.path()).unwrap();

        assert_eq!(report.concept_count, 1);
        assert_eq!(report.okf_version, "0.2");
        assert_eq!(report.warnings.len(), 1);
    }

    #[test]
    fn bare_verification_is_normalized_without_losing_metadata() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("index.md"), valid_index()).unwrap();
        fs::write(
            temp.path().join("concept.md"),
            b"---\ntype: Reference\nverified:\n  by: human:owner\n  at: 2026-08-12T12:00:00Z\n---\n",
        )
        .unwrap();

        let report = OkfImportValidator::validate_directory(temp.path()).unwrap();

        assert!(
            report.concepts[0].verified.as_ref().is_some_and(|value| {
                value.as_sequence().is_some_and(|values| values.len() == 1)
            })
        );
    }

    #[test]
    fn directory_rejects_missing_concept_type() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("index.md"), valid_index()).unwrap();
        fs::write(
            temp.path().join("invalid.md"),
            b"---\ntitle: Missing\n---\n",
        )
        .unwrap();

        assert!(OkfImportValidator::validate_directory(temp.path()).is_err());
    }

    #[test]
    fn zip_rejects_traversal() {
        let cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        writer
            .start_file("../outside.md", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(valid_concept()).unwrap();
        let bytes = writer.finish().unwrap().into_inner();

        assert!(OkfImportValidator::validate_zip(Cursor::new(bytes)).is_err());
    }

    #[test]
    fn zip_rejects_symbolic_links() {
        let cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        writer
            .add_symlink(
                "concept.md",
                "target.md",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        let bytes = writer.finish().unwrap().into_inner();

        assert!(OkfImportValidator::validate_zip(Cursor::new(bytes)).is_err());
    }

    #[test]
    fn materialization_copies_a_valid_bundle_and_revalidates_it() {
        let source = tempfile::tempdir().unwrap();
        let parent = tempfile::tempdir().unwrap();
        let staging = parent.path().join("staging");
        fs::write(source.path().join("index.md"), valid_index()).unwrap();
        fs::write(source.path().join("concept.md"), valid_concept()).unwrap();

        let report = OkfImportValidator::materialize_path(source.path(), &staging).unwrap();

        assert_eq!(report.concept_count, 1);
        assert_eq!(
            fs::read(staging.join("concept.md")).unwrap(),
            valid_concept()
        );
    }

    #[test]
    fn failed_materialization_leaves_no_staging_directory() {
        let source = tempfile::tempdir().unwrap();
        let parent = tempfile::tempdir().unwrap();
        let staging = parent.path().join("staging");
        fs::write(source.path().join("index.md"), valid_index()).unwrap();
        fs::write(source.path().join("invalid.md"), b"missing frontmatter").unwrap();

        assert!(OkfImportValidator::materialize_path(source.path(), &staging).is_err());
        assert!(!staging.exists());
    }
}
