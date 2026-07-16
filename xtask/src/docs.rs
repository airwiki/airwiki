use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};

const REQUIRED_ROOT_FILES: [&str; 10] = [
    "AGENTS.md",
    "CHANGELOG.md",
    "CODE_REVIEW.md",
    "CODE_OF_CONDUCT.md",
    "CONTRIBUTING.md",
    "LICENSE",
    "PLANS.md",
    "README.md",
    "SECURITY.md",
    "THIRD_PARTY_NOTICES.md",
];
const ADR_STATUSES: [&str; 4] = ["Proposed", "Accepted", "Superseded", "Rejected"];
const ADR_REQUIRED_SECTIONS: [&str; 4] = [
    "## Context",
    "## Decision",
    "## Consequences",
    "## Rejected alternatives",
];
const ACTIVE_WORKFLOWS: [&str; 3] = ["ci.yml", "dco.yml", "package-pilot.yml"];
const ARCHIVED_WORKFLOWS: [&str; 3] = [
    "README.md",
    "promote-stable.yml.disabled",
    "release-candidate.yml.disabled",
];
const STALE_CARGO_DENY_COMMAND: &str = "cargo deny check --locked";

pub(crate) fn check(repository_root: &Path) -> Result<()> {
    ensure!(
        repository_root.is_dir(),
        "repository root does not exist: {}",
        repository_root.display()
    );

    let mut issues = Vec::new();
    check_required_files(repository_root, &mut issues);
    check_retired_plans(repository_root, &mut issues);

    let markdown_files = collect_markdown_files(repository_root)?;
    check_markdown_files(repository_root, &markdown_files, &mut issues)?;
    check_adrs(repository_root, &mut issues)?;
    check_workflows(repository_root, &mut issues)?;

    if issues.is_empty() {
        println!(
            "Documentation checks passed ({} Markdown files).",
            markdown_files.len()
        );
        return Ok(());
    }

    issues.sort();
    issues.dedup();
    bail!(
        "documentation checks failed ({} issue(s)):\n- {}",
        issues.len(),
        issues.join("\n- ")
    )
}

fn check_required_files(repository_root: &Path, issues: &mut Vec<String>) {
    for relative_path in REQUIRED_ROOT_FILES {
        if !repository_root.join(relative_path).is_file() {
            issues.push(format!("missing required root file `{relative_path}`"));
        }
    }
}

fn check_retired_plans(repository_root: &Path, issues: &mut Vec<String>) {
    if repository_root.join("docs/superpowers").exists() {
        issues.push("retired plan tree `docs/superpowers` must not exist".to_owned());
    }
}

fn collect_markdown_files(repository_root: &Path) -> Result<Vec<PathBuf>> {
    let mut directories = vec![repository_root.to_path_buf()];
    let mut files = Vec::new();

    while let Some(directory) = directories.pop() {
        let entries = fs::read_dir(&directory)
            .with_context(|| format!("failed to read `{}`", directory.display()))?;
        for entry in entries {
            let entry =
                entry.with_context(|| format!("failed to inspect `{}`", directory.display()))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .with_context(|| format!("failed to inspect `{}`", path.display()))?;

            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                if !should_skip_directory(repository_root, &path) {
                    directories.push(path);
                }
                continue;
            }
            if file_type.is_file() && path.extension().is_some_and(|value| value == "md") {
                files.push(path);
            }
        }
    }

    files.sort();
    Ok(files)
}

fn should_skip_directory(repository_root: &Path, directory: &Path) -> bool {
    let Ok(relative) = directory.strip_prefix(repository_root) else {
        return true;
    };
    let Some(first) = relative.components().next() else {
        return false;
    };
    matches!(
        first.as_os_str().to_str(),
        Some(".agents" | ".claude" | ".git" | ".superpowers" | "target")
    ) || relative == Path::new("docs/superpowers")
}

fn check_markdown_files(
    repository_root: &Path,
    markdown_files: &[PathBuf],
    issues: &mut Vec<String>,
) -> Result<()> {
    for path in markdown_files {
        let content = fs::read_to_string(path)
            .with_context(|| format!("failed to read `{}`", path.display()))?;
        let relative_path = display_relative(repository_root, path);

        if content.contains(STALE_CARGO_DENY_COMMAND) {
            issues.push(format!(
                "{relative_path}: replace stale `{STALE_CARGO_DENY_COMMAND}` with `cargo deny --locked check`"
            ));
        }

        for target in markdown_link_targets(&content) {
            let Some(local_target) = local_link_target(&target) else {
                continue;
            };
            match resolve_local_link(repository_root, path, local_target) {
                Ok(resolved) if !resolved.exists() => issues.push(format!(
                    "{relative_path}: local link `{target}` does not exist"
                )),
                Ok(_) => {}
                Err(message) => issues.push(format!("{relative_path}: link `{target}` {message}")),
            }
        }
    }
    Ok(())
}

fn markdown_link_targets(content: &str) -> Vec<String> {
    let mut targets = Vec::new();
    let mut in_fence = false;

    for line in content.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }

        extract_inline_link_targets(line, &mut targets);
        if let Some(target) = reference_definition_target(trimmed) {
            targets.push(target.to_owned());
        }
    }

    targets
}

fn extract_inline_link_targets(line: &str, targets: &mut Vec<String>) {
    let bytes = line.as_bytes();
    let mut cursor = 0;
    let mut in_code = false;

    while cursor < bytes.len() {
        if bytes[cursor] == b'`' {
            in_code = !in_code;
            cursor += 1;
            continue;
        }
        if !in_code && bytes[cursor..].starts_with(b"](") {
            let destination_start = cursor + 2;
            let Some(destination_end) = bytes[destination_start..]
                .iter()
                .position(|byte| *byte == b')')
                .map(|offset| destination_start + offset)
            else {
                break;
            };
            if let Some(destination) =
                markdown_destination(&line[destination_start..destination_end])
            {
                targets.push(destination.to_owned());
            }
            cursor = destination_end + 1;
            continue;
        }
        cursor += 1;
    }
}

fn reference_definition_target(line: &str) -> Option<&str> {
    if !line.starts_with('[') {
        return None;
    }
    let marker = line.find("]:")?;
    markdown_destination(&line[marker + 2..])
}

fn markdown_destination(value: &str) -> Option<&str> {
    let value = value.trim();
    if let Some(value) = value.strip_prefix('<') {
        return value.split_once('>').map(|(destination, _)| destination);
    }
    value
        .split_ascii_whitespace()
        .next()
        .filter(|value| !value.is_empty())
}

fn local_link_target(target: &str) -> Option<&str> {
    let target = target.trim();
    if target.is_empty()
        || target.starts_with('#')
        || target.starts_with("//")
        || has_uri_scheme(target)
    {
        return None;
    }

    let end = target
        .char_indices()
        .find_map(|(index, character)| matches!(character, '#' | '?').then_some(index))
        .unwrap_or(target.len());
    let target = &target[..end];
    (!target.is_empty()).then_some(target)
}

fn has_uri_scheme(target: &str) -> bool {
    let Some((scheme, _)) = target.split_once(':') else {
        return false;
    };
    let mut characters = scheme.chars();
    characters
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic())
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        })
}

fn resolve_local_link(
    repository_root: &Path,
    source: &Path,
    target: &str,
) -> std::result::Result<PathBuf, &'static str> {
    let source_directory = source.parent().ok_or("has no parent directory")?;
    let relative_source_directory = source_directory
        .strip_prefix(repository_root)
        .map_err(|_| "is outside the repository")?;
    let mut components: Vec<OsString> = if target.starts_with('/') {
        Vec::new()
    } else {
        relative_source_directory
            .components()
            .filter_map(|component| match component {
                Component::Normal(value) => Some(value.to_os_string()),
                _ => None,
            })
            .collect()
    };

    for component in Path::new(target).components() {
        match component {
            Component::CurDir | Component::RootDir => {}
            Component::Normal(value) => components.push(value.to_os_string()),
            Component::ParentDir => {
                if components.pop().is_none() {
                    return Err("escapes the repository");
                }
            }
            Component::Prefix(_) => return Err("uses an unsupported path prefix"),
        }
    }

    let mut resolved = repository_root.to_path_buf();
    resolved.extend(components);
    Ok(resolved)
}

fn check_adrs(repository_root: &Path, issues: &mut Vec<String>) -> Result<()> {
    let adr_directory = repository_root.join("docs/adr");
    let index_path = adr_directory.join("README.md");
    let indexed_adrs = if index_path.is_file() {
        let index_content = fs::read_to_string(&index_path)
            .with_context(|| format!("failed to read `{}`", index_path.display()))?;
        adr_index_entries(&index_content, issues)
    } else {
        issues.push("missing ADR index `docs/adr/README.md`".to_owned());
        BTreeMap::new()
    };

    let mut records = Vec::new();
    if adr_directory.is_dir() {
        for entry in fs::read_dir(&adr_directory)
            .with_context(|| format!("failed to read `{}`", adr_directory.display()))?
        {
            let entry = entry.context("failed to inspect an ADR directory entry")?;
            let path = entry.path();
            if !entry
                .file_type()
                .with_context(|| format!("failed to inspect `{}`", path.display()))?
                .is_file()
            {
                continue;
            }
            let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if file_name == "README.md" || path.extension().is_none_or(|value| value != "md") {
                continue;
            }
            match adr_number(file_name) {
                Some(number) => records.push((number, file_name.to_owned(), path)),
                None => issues.push(format!(
                    "ADR file `docs/adr/{file_name}` must use `NNNN-title.md`"
                )),
            }
        }
    } else {
        issues.push("missing ADR directory `docs/adr`".to_owned());
    }

    records.sort_by_key(|(number, _, _)| *number);
    if records.is_empty() {
        issues.push("ADR directory contains no numbered decisions".to_owned());
        return Ok(());
    }

    for (index, (number, file_name, path)) in records.iter().enumerate() {
        let expected_number = u32::try_from(index + 1).context("ADR count exceeds u32")?;
        if *number != expected_number {
            issues.push(format!(
                "ADR numbering is not contiguous: expected {expected_number:04}, found {number:04}"
            ));
        }
        let metadata = check_adr_file(repository_root, *number, path, issues)?;
        if let Some(indexed) = indexed_adrs.get(file_name) {
            check_adr_index_metadata(file_name, &metadata, indexed, issues);
        } else {
            issues.push(format!("ADR index does not link `{file_name}`"));
        }
    }

    let record_files: BTreeSet<&str> = records
        .iter()
        .map(|(_, file_name, _)| file_name.as_str())
        .collect();
    for file_name in indexed_adrs.keys() {
        if !record_files.contains(file_name.as_str()) {
            issues.push(format!("ADR index links unknown decision `{file_name}`"));
        }
    }

    Ok(())
}

#[derive(Debug)]
struct AdrIndexEntry {
    status: String,
    date: String,
}

fn adr_index_entries(content: &str, issues: &mut Vec<String>) -> BTreeMap<String, AdrIndexEntry> {
    let mut entries = BTreeMap::new();
    for line in content.lines() {
        let columns: Vec<&str> = line.split('|').map(str::trim).collect();
        if columns.len() < 6 {
            continue;
        }
        let Some(target) = markdown_link_targets(columns[1]).into_iter().next() else {
            continue;
        };
        let Some(file_name) = local_link_target(&target)
            .and_then(|target| Path::new(target).file_name())
            .and_then(|value| value.to_str())
            .filter(|file_name| adr_number(file_name).is_some())
        else {
            continue;
        };
        let entry = AdrIndexEntry {
            status: columns[3].to_owned(),
            date: columns[4].to_owned(),
        };
        if entries.insert(file_name.to_owned(), entry).is_some() {
            issues.push(format!(
                "ADR index contains duplicate decision `{file_name}`"
            ));
        }
    }
    entries
}

fn adr_number(file_name: &str) -> Option<u32> {
    let (prefix, remainder) = file_name.split_once('-')?;
    if prefix.len() != 4 || !remainder.ends_with(".md") {
        return None;
    }
    prefix.parse().ok()
}

fn check_adr_file(
    repository_root: &Path,
    number: u32,
    path: &Path,
    issues: &mut Vec<String>,
) -> Result<AdrMetadata> {
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read `{}`", path.display()))?;
    let relative_path = display_relative(repository_root, path);
    let expected_heading = format!("# ADR {number:04}:");
    if !content
        .lines()
        .next()
        .is_some_and(|line| line.starts_with(&expected_heading))
    {
        issues.push(format!(
            "{relative_path}: first line must start with `{expected_heading}`"
        ));
    }

    let status = metadata_value(&content, "Status").map(str::to_owned);
    match status.as_deref() {
        Some(status) if ADR_STATUSES.contains(&status) => {}
        Some(status) => issues.push(format!(
            "{relative_path}: unsupported ADR status `{status}`"
        )),
        None => issues.push(format!("{relative_path}: missing `- Status:` metadata")),
    }
    let date = metadata_value(&content, "Date").map(str::to_owned);
    match date.as_deref() {
        Some(date) if valid_iso_date(date) => {}
        Some(date) => issues.push(format!(
            "{relative_path}: ADR date `{date}` must use YYYY-MM-DD"
        )),
        None => issues.push(format!("{relative_path}: missing `- Date:` metadata")),
    }

    for section in ADR_REQUIRED_SECTIONS {
        if !content.lines().any(|line| line.trim() == section) {
            issues.push(format!(
                "{relative_path}: missing required section `{section}`"
            ));
        }
    }

    Ok(AdrMetadata { status, date })
}

#[derive(Debug)]
struct AdrMetadata {
    status: Option<String>,
    date: Option<String>,
}

fn check_adr_index_metadata(
    file_name: &str,
    metadata: &AdrMetadata,
    indexed: &AdrIndexEntry,
    issues: &mut Vec<String>,
) {
    if metadata.status.as_deref() != Some(indexed.status.as_str()) {
        issues.push(format!(
            "ADR index status for `{file_name}` is `{}`, expected `{}`",
            indexed.status,
            metadata.status.as_deref().unwrap_or("missing")
        ));
    }
    if metadata.date.as_deref() != Some(indexed.date.as_str()) {
        issues.push(format!(
            "ADR index date for `{file_name}` is `{}`, expected `{}`",
            indexed.date,
            metadata.date.as_deref().unwrap_or("missing")
        ));
    }
}

fn metadata_value<'a>(content: &'a str, field: &str) -> Option<&'a str> {
    let prefix = format!("- {field}:");
    content
        .lines()
        .find_map(|line| line.trim().strip_prefix(&prefix).map(str::trim))
        .filter(|value| !value.is_empty())
}

fn valid_iso_date(date: &str) -> bool {
    let bytes = date.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
}

fn check_workflows(repository_root: &Path, issues: &mut Vec<String>) -> Result<()> {
    let workflows_directory = repository_root.join(".github/workflows");
    let mut active = BTreeSet::new();
    if workflows_directory.is_dir() {
        for entry in fs::read_dir(&workflows_directory)
            .with_context(|| format!("failed to read `{}`", workflows_directory.display()))?
        {
            let entry = entry.context("failed to inspect an active workflow")?;
            if !entry
                .file_type()
                .with_context(|| format!("failed to inspect `{}`", entry.path().display()))?
                .is_file()
            {
                continue;
            }
            active.insert(entry.file_name().to_string_lossy().into_owned());
        }
    } else {
        issues.push("missing active workflow directory `.github/workflows`".to_owned());
    }

    let expected: BTreeSet<String> = ACTIVE_WORKFLOWS
        .iter()
        .map(|value| (*value).to_owned())
        .collect();
    for missing in expected.difference(&active) {
        issues.push(format!(
            "missing active workflow `.github/workflows/{missing}`"
        ));
    }
    for unexpected in active.difference(&expected) {
        issues.push(format!(
            "unexpected active workflow `.github/workflows/{unexpected}`; signed release workflows must remain archived"
        ));
    }

    let archive_directory = repository_root.join("docs/archive/release-workflows");
    for file_name in ARCHIVED_WORKFLOWS {
        if !archive_directory.join(file_name).is_file() {
            issues.push(format!(
                "missing archived release workflow `docs/archive/release-workflows/{file_name}`"
            ));
        }
    }
    if archive_directory.is_dir() {
        for entry in fs::read_dir(&archive_directory)
            .with_context(|| format!("failed to read `{}`", archive_directory.display()))?
        {
            let entry = entry.context("failed to inspect an archived workflow")?;
            if matches!(
                entry.path().extension().and_then(|value| value.to_str()),
                Some("yml" | "yaml")
            ) {
                issues.push(format!(
                    "archived workflow `{}` still uses an executable workflow extension",
                    display_relative(repository_root, &entry.path())
                ));
            }
        }
    }

    Ok(())
}

fn display_relative(repository_root: &Path, path: &Path) -> String {
    path.strip_prefix(repository_root)
        .unwrap_or(path)
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        directory: tempfile::TempDir,
    }

    impl Fixture {
        fn valid() -> Self {
            let directory = tempfile::tempdir().unwrap();
            let fixture = Self { directory };
            for relative_path in REQUIRED_ROOT_FILES {
                fixture.write(relative_path, "# Document\n");
            }
            fixture.write(
                "docs/adr/README.md",
                "# Architecture decisions\n\n| Number | Decision | Status | Date | Relationship |\n| --- | --- | --- | --- | --- |\n| [0001](0001-first-decision.md) | First decision | Accepted | 2026-07-15 | — |\n",
            );
            fixture.write(
                "docs/adr/0001-first-decision.md",
                "# ADR 0001: First decision\n\n- Status: Accepted\n- Date: 2026-07-15\n\n## Context\n\nContext.\n\n## Decision\n\nDecision.\n\n## Consequences\n\nConsequences.\n\n## Rejected alternatives\n\nAlternatives.\n",
            );
            for file_name in ACTIVE_WORKFLOWS {
                fixture.write(&format!(".github/workflows/{file_name}"), "name: Test\n");
            }
            for file_name in ARCHIVED_WORKFLOWS {
                fixture.write(
                    &format!("docs/archive/release-workflows/{file_name}"),
                    "archived\n",
                );
            }
            fixture
        }

        fn root(&self) -> &Path {
            self.directory.path()
        }

        fn write(&self, relative_path: &str, content: &str) {
            let path = self.root().join(relative_path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, content).unwrap();
        }
    }

    #[test]
    fn check_accepts_existing_local_link() {
        let fixture = Fixture::valid();
        fixture.write("README.md", "[Architecture](docs/architecture.md)\n");
        fixture.write("docs/architecture.md", "# Architecture\n");

        assert!(check(fixture.root()).is_ok());
    }

    #[test]
    fn check_rejects_missing_local_link() {
        let fixture = Fixture::valid();
        fixture.write("README.md", "[Missing](docs/missing.md)\n");

        let error = check(fixture.root()).unwrap_err().to_string();

        assert!(error.contains("local link `docs/missing.md` does not exist"));
    }

    #[test]
    fn check_ignores_external_and_fragment_only_links() {
        let fixture = Fixture::valid();
        fixture.write(
            "README.md",
            "[External](https://example.com/docs) [Section](#section)\n",
        );

        assert!(check(fixture.root()).is_ok());
    }

    #[test]
    fn check_resolves_the_file_part_of_a_fragment_link() {
        let fixture = Fixture::valid();
        fixture.write("README.md", "[Architecture](docs/architecture.md#scope)\n");
        fixture.write("docs/architecture.md", "# Architecture\n\n## Scope\n");

        assert!(check(fixture.root()).is_ok());
    }

    #[test]
    fn check_rejects_link_that_escapes_repository() {
        let fixture = Fixture::valid();
        fixture.write("README.md", "[Outside](../outside.md)\n");

        let error = check(fixture.root()).unwrap_err().to_string();

        assert!(error.contains("escapes the repository"));
    }

    #[test]
    fn check_rejects_adr_without_required_metadata() {
        let fixture = Fixture::valid();
        fixture.write(
            "docs/adr/0001-first-decision.md",
            "# ADR 0001: First decision\n\n- Date: 2026-07-15\n",
        );

        let error = check(fixture.root()).unwrap_err().to_string();

        assert!(error.contains("missing `- Status:` metadata"));
    }

    #[test]
    fn check_rejects_adr_with_invalid_status() {
        let fixture = Fixture::valid();
        fixture.write(
            "docs/adr/0001-first-decision.md",
            "# ADR 0001: First decision\n\n- Status: Draft\n- Date: 2026-07-15\n",
        );

        let error = check(fixture.root()).unwrap_err().to_string();

        assert!(error.contains("unsupported ADR status `Draft`"));
    }

    #[test]
    fn check_rejects_adr_without_required_section() {
        let fixture = Fixture::valid();
        fixture.write(
            "docs/adr/0001-first-decision.md",
            "# ADR 0001: First decision\n\n- Status: Accepted\n- Date: 2026-07-15\n\n## Context\n\nContext.\n\n## Decision\n\nDecision.\n\n## Consequences\n\nConsequences.\n",
        );

        let error = check(fixture.root()).unwrap_err().to_string();

        assert!(error.contains("missing required section `## Rejected alternatives`"));
    }

    #[test]
    fn check_rejects_adr_index_status_mismatch() {
        let fixture = Fixture::valid();
        fixture.write(
            "docs/adr/README.md",
            "# Architecture decisions\n\n| Number | Decision | Status | Date | Relationship |\n| --- | --- | --- | --- | --- |\n| [0001](0001-first-decision.md) | First decision | Proposed | 2026-07-15 | — |\n",
        );

        let error = check(fixture.root()).unwrap_err().to_string();

        assert!(error.contains("index status for `0001-first-decision.md` is `Proposed`"));
    }

    #[test]
    fn check_rejects_adr_index_date_mismatch() {
        let fixture = Fixture::valid();
        fixture.write(
            "docs/adr/README.md",
            "# Architecture decisions\n\n| Number | Decision | Status | Date | Relationship |\n| --- | --- | --- | --- | --- |\n| [0001](0001-first-decision.md) | First decision | Accepted | 2026-07-16 | — |\n",
        );

        let error = check(fixture.root()).unwrap_err().to_string();

        assert!(error.contains("index date for `0001-first-decision.md` is `2026-07-16`"));
    }

    #[test]
    fn check_rejects_duplicate_adr_index_entry() {
        let fixture = Fixture::valid();
        fixture.write(
            "docs/adr/README.md",
            "# Architecture decisions\n\n| Number | Decision | Status | Date | Relationship |\n| --- | --- | --- | --- | --- |\n| [0001](0001-first-decision.md) | First decision | Accepted | 2026-07-15 | — |\n| [0001](0001-first-decision.md) | First decision | Accepted | 2026-07-15 | — |\n",
        );

        let error = check(fixture.root()).unwrap_err().to_string();

        assert!(error.contains("index contains duplicate decision `0001-first-decision.md`"));
    }

    #[test]
    fn check_rejects_unknown_adr_index_entry() {
        let fixture = Fixture::valid();
        fixture.write(
            "docs/adr/README.md",
            "# Architecture decisions\n\n| Number | Decision | Status | Date | Relationship |\n| --- | --- | --- | --- | --- |\n| [0001](0001-first-decision.md) | First decision | Accepted | 2026-07-15 | — |\n| [0002](0002-missing-decision.md) | Missing decision | Proposed | 2026-07-16 | — |\n",
        );

        let error = check(fixture.root()).unwrap_err().to_string();

        assert!(error.contains("index links unknown decision `0002-missing-decision.md`"));
    }

    #[test]
    fn check_rejects_adr_missing_from_index() {
        let fixture = Fixture::valid();
        fixture.write(
            "docs/adr/0002-second-decision.md",
            "# ADR 0002: Second decision\n\n- Status: Proposed\n- Date: 2026-07-15\n",
        );

        let error = check(fixture.root()).unwrap_err().to_string();

        assert!(error.contains("ADR index does not link `0002-second-decision.md`"));
    }

    #[test]
    fn check_rejects_stale_cargo_deny_command() {
        let fixture = Fixture::valid();
        fixture.write("README.md", "Run `cargo deny check --locked`.\n");

        let error = check(fixture.root()).unwrap_err().to_string();

        assert!(error.contains("replace stale `cargo deny check --locked`"));
    }

    #[test]
    fn check_rejects_retired_superpowers_tree() {
        let fixture = Fixture::valid();
        fixture.write("docs/superpowers/plan.md", "# Old plan\n");

        let error = check(fixture.root()).unwrap_err().to_string();

        assert!(error.contains("retired plan tree `docs/superpowers` must not exist"));
    }

    #[test]
    fn check_rejects_active_signed_release_workflow() {
        let fixture = Fixture::valid();
        fixture.write(".github/workflows/release-candidate.yml", "name: Release\n");

        let error = check(fixture.root()).unwrap_err().to_string();

        assert!(error.contains("unexpected active workflow"));
    }
}
