use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use airwiki_types::MAX_HEADING_OR_PAGE_CHARS;
use anyhow::{Context, Result, bail};
use lopdf::Document;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat {
    Markdown,
    Pdf,
}

impl SourceFormat {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Markdown => "markdown",
            Self::Pdf => "pdf",
        }
    }

    pub fn from_path(path: &Path) -> Option<Self> {
        match path
            .extension()?
            .to_string_lossy()
            .to_ascii_lowercase()
            .as_str()
        {
            "md" | "markdown" => Some(Self::Markdown),
            "pdf" => Some(Self::Pdf),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct IngestLimits {
    pub max_bytes: u64,
    pub max_pdf_pages: usize,
    pub max_characters: usize,
}

impl Default for IngestLimits {
    fn default() -> Self {
        Self {
            max_bytes: 50 * 1024 * 1024,
            max_pdf_pages: 500,
            max_characters: 2_000_000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FileCandidate {
    pub path: PathBuf,
    pub format: SourceFormat,
    pub byte_size: u64,
}

#[derive(Debug, Clone)]
pub struct FileDiscoveryIssue {
    pub path: PathBuf,
    pub error: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileDiscoveryStatus {
    Complete,
    Incomplete,
}

#[derive(Debug, Clone)]
pub struct FileDiscovery {
    pub candidates: Vec<FileCandidate>,
    pub issues: Vec<FileDiscoveryIssue>,
    pub status: FileDiscoveryStatus,
}

impl Default for FileDiscovery {
    fn default() -> Self {
        Self {
            candidates: Vec::new(),
            issues: Vec::new(),
            status: FileDiscoveryStatus::Complete,
        }
    }
}

impl FileDiscovery {
    pub fn is_complete(&self) -> bool {
        self.status == FileDiscoveryStatus::Complete
    }
}

#[derive(Debug, Clone)]
pub struct ExtractedSection {
    /// Markdown heading or `Página N` for PDF content.
    pub heading_or_page: String,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct ExtractedDocument {
    pub format: SourceFormat,
    pub sections: Vec<ExtractedSection>,
    pub page_count: usize,
    pub character_count: usize,
}

impl ExtractedDocument {
    pub fn plain_text(&self) -> String {
        self.sections
            .iter()
            .map(|section| format!("{}\n{}", section.heading_or_page, section.text))
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// Produces a deterministic, stratified view for document-level metadata.
    /// Search chunks still cover the complete source; this only prevents a long
    /// document from requiring dozens of serial LLM summary calls before it can
    /// enter human review.
    pub fn representative_text(&self, max_bytes: usize) -> String {
        const MAX_FRAGMENTS: usize = 8;
        const PREAMBLE: &str = "Muestra estratificada de un documento extenso; los fragmentos conservan su orden original.";

        let full = self.plain_text();
        if full.len() <= max_bytes {
            return full;
        }
        if max_bytes == 0 {
            return String::new();
        }

        let mut output = utf8_prefix(PREAMBLE, max_bytes).to_owned();
        if output.len() == max_bytes {
            return output;
        }
        let available_after_preamble = max_bytes - output.len();
        let fragment_count = MAX_FRAGMENTS
            .min((available_after_preamble / 64).max(1))
            .min(full.len());
        let labels = (0..fragment_count)
            .map(|index| format!("\n\n[Fragmento {}/{}]\n", index + 1, fragment_count))
            .collect::<Vec<_>>();
        let label_bytes = labels.iter().map(String::len).sum::<usize>();
        if label_bytes >= available_after_preamble {
            output.push_str(utf8_prefix(&labels[0], max_bytes - output.len()));
            return output;
        }

        let content_budget = available_after_preamble - label_bytes;
        let base_window = content_budget / fragment_count;
        let extra = content_budget % fragment_count;
        for (index, label) in labels.into_iter().enumerate() {
            output.push_str(&label);
            let window_bytes = base_window + usize::from(index < extra);
            let raw_start = if fragment_count == 1 {
                0
            } else {
                index * full.len().saturating_sub(window_bytes) / (fragment_count - 1)
            };
            let start = next_char_boundary(&full, raw_start.min(full.len()));
            let end = previous_char_boundary(&full, (start + window_bytes).min(full.len()));
            if end > start {
                output.push_str(full[start..end].trim());
            }
        }
        debug_assert!(output.len() <= max_bytes);
        output
    }
}

fn utf8_prefix(text: &str, max_bytes: usize) -> &str {
    &text[..previous_char_boundary(text, max_bytes.min(text.len()))]
}

fn previous_char_boundary(text: &str, mut index: usize) -> usize {
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn next_char_boundary(text: &str, mut index: usize) -> usize {
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

#[derive(Debug, Clone)]
pub struct ChunkDraft {
    pub ordinal: u32,
    pub heading_or_page: String,
    pub text: String,
    pub token_count: usize,
}

/// Keeps the chunker independent from a concrete embeddings runtime.
pub trait Tokenizer: Send + Sync {
    fn encode(&self, text: &str) -> Result<Vec<String>>;

    fn decode(&self, tokens: &[String]) -> Result<String> {
        Ok(tokens.join(" "))
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct WhitespaceTokenizer;

impl Tokenizer for WhitespaceTokenizer {
    fn encode(&self, text: &str) -> Result<Vec<String>> {
        Ok(text.split_whitespace().map(ToOwned::to_owned).collect())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Chunker {
    pub target_tokens: usize,
    pub overlap_tokens: usize,
}

impl Default for Chunker {
    fn default() -> Self {
        Self {
            target_tokens: 384,
            overlap_tokens: 64,
        }
    }
}

impl Chunker {
    pub fn new(target_tokens: usize, overlap_tokens: usize) -> Result<Self> {
        if target_tokens == 0 || overlap_tokens >= target_tokens {
            bail!("chunk target must be positive and overlap smaller than target");
        }
        Ok(Self {
            target_tokens,
            overlap_tokens,
        })
    }

    pub fn chunk(
        &self,
        document: &ExtractedDocument,
        tokenizer: &dyn Tokenizer,
    ) -> Result<Vec<ChunkDraft>> {
        let mut chunks = Vec::new();
        for section in &document.sections {
            let tokens = tokenizer.encode(&section.text)?;
            if tokens.is_empty() {
                continue;
            }
            // Extractors already bound headings, but Chunker is public and may
            // receive a caller-built document. Normalize once per section so a
            // hostile heading cannot be cloned in full into every chunk.
            let heading_or_page = bounded_heading_or_page(&section.heading_or_page);
            let step = self.target_tokens - self.overlap_tokens;
            let mut start = 0;
            while start < tokens.len() {
                let end = (start + self.target_tokens).min(tokens.len());
                let text = tokenizer.decode(&tokens[start..end])?;
                chunks.push(ChunkDraft {
                    ordinal: u32::try_from(chunks.len()).context("too many chunks")?,
                    heading_or_page: heading_or_page.clone(),
                    text,
                    token_count: end - start,
                });
                if end == tokens.len() {
                    break;
                }
                start += step;
            }
        }
        Ok(chunks)
    }
}

pub fn discover_files(root: impl AsRef<Path>, limits: IngestLimits) -> Result<Vec<FileCandidate>> {
    let discovery = discover_files_with_issues(root, limits)?;
    if !discovery.is_complete() {
        bail!("source traversal is incomplete");
    }
    Ok(discovery.candidates)
}

pub fn discover_files_with_issues(
    root: impl AsRef<Path>,
    limits: IngestLimits,
) -> Result<FileDiscovery> {
    let root = root.as_ref();
    if !root.is_dir() {
        bail!("source folder does not exist: {}", root.display());
    }
    let mut discovery = FileDiscovery::default();
    for entry in WalkDir::new(root).follow_links(false).into_iter() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                let path = error.path().unwrap_or(root).to_path_buf();
                let message = error.to_string();
                tracing::warn!(
                    io_error_kind = ?error.io_error().map(std::io::Error::kind),
                    "skipping unreadable source entry"
                );
                discovery.status = FileDiscoveryStatus::Incomplete;
                discovery.issues.push(FileDiscoveryIssue {
                    path,
                    error: format!("source traversal is incomplete: {message}"),
                });
                continue;
            }
        };
        let relative = entry.path().strip_prefix(root).unwrap_or(entry.path());
        if entry.depth() > 0 && is_hidden_or_temporary(relative) {
            continue;
        }
        let file_type = entry.file_type();
        if !file_type.is_file() || file_type.is_symlink() {
            continue;
        }
        let Some(format) = SourceFormat::from_path(entry.path()) else {
            continue;
        };
        let byte_size = match entry.metadata() {
            Ok(metadata) => metadata.len(),
            Err(error) => {
                tracing::warn!(
                    io_error_kind = ?error.io_error().map(std::io::Error::kind),
                    "skipping source whose metadata is unavailable"
                );
                discovery.status = FileDiscoveryStatus::Incomplete;
                discovery.issues.push(FileDiscoveryIssue {
                    path: entry.path().to_path_buf(),
                    error: format!("source metadata is unavailable: {error}"),
                });
                continue;
            }
        };
        if byte_size > limits.max_bytes {
            tracing::warn!(byte_size, "source exceeds size limit");
            discovery.issues.push(FileDiscoveryIssue {
                path: entry.path().to_path_buf(),
                error: format!(
                    "source exceeds the {} byte limit (actual size: {byte_size} bytes)",
                    limits.max_bytes
                ),
            });
            continue;
        }
        discovery.candidates.push(FileCandidate {
            path: entry.path().to_path_buf(),
            format,
            byte_size,
        });
    }
    discovery
        .candidates
        .sort_by(|left, right| left.path.cmp(&right.path));
    discovery
        .issues
        .sort_by(|left, right| left.path.cmp(&right.path));
    Ok(discovery)
}

pub fn sha256_file(path: impl AsRef<Path>) -> Result<String> {
    let path = path.as_ref();
    let mut reader = BufReader::new(
        File::open(path).with_context(|| format!("could not open {}", path.display()))?,
    );
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .with_context(|| format!("could not read {}", path.display()))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub fn extract_file(
    path: impl AsRef<Path>,
    format: SourceFormat,
    limits: IngestLimits,
) -> Result<ExtractedDocument> {
    let path = path.as_ref();
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("could not inspect {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        bail!("symbolic links are not accepted");
    }
    if metadata.len() > limits.max_bytes {
        bail!("source exceeds the {} byte limit", limits.max_bytes);
    }
    match format {
        SourceFormat::Markdown => extract_markdown(path, limits),
        SourceFormat::Pdf => extract_pdf(path, limits),
    }
}

fn extract_markdown(path: &Path, limits: IngestLimits) -> Result<ExtractedDocument> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("Markdown is not valid UTF-8: {}", path.display()))?;
    if text.chars().count() > limits.max_characters {
        bail!(
            "document exceeds the {} character limit",
            limits.max_characters
        );
    }
    let mut sections = Vec::new();
    let mut heading = "Documento".to_owned();
    let mut body = Vec::new();
    for line in text.lines() {
        if let Some(next_heading) = markdown_heading(line) {
            push_section(&mut sections, &heading, &body.join("\n"));
            heading = next_heading;
            body.clear();
        } else {
            body.push(line);
        }
    }
    push_section(&mut sections, &heading, &body.join("\n"));
    if sections.is_empty() && !text.trim().is_empty() {
        sections.push(ExtractedSection {
            heading_or_page: "Documento".into(),
            text: text.trim().to_owned(),
        });
    }
    Ok(ExtractedDocument {
        format: SourceFormat::Markdown,
        sections,
        page_count: 0,
        character_count: text.chars().count(),
    })
}

fn extract_pdf(path: &Path, limits: IngestLimits) -> Result<ExtractedDocument> {
    // lopdf may transparently decrypt documents whose user password is empty;
    // inspect metadata first so the MVP's "no encrypted PDF" rule remains
    // absolute rather than password-dependent.
    let metadata = Document::load_metadata(path)
        .with_context(|| format!("could not inspect PDF metadata {}", path.display()))?;
    if metadata.encrypted {
        bail!("encrypted PDFs are not supported");
    }
    let document =
        Document::load(path).with_context(|| format!("could not parse PDF {}", path.display()))?;
    if document.is_encrypted() {
        bail!("encrypted PDFs are not supported");
    }
    let pages = document.get_pages();
    if pages.len() > limits.max_pdf_pages {
        bail!(
            "PDF has {} pages; maximum is {}",
            pages.len(),
            limits.max_pdf_pages
        );
    }
    let mut sections = Vec::new();
    let mut characters = 0_usize;
    for page_number in pages.keys().copied() {
        let page_text = document
            .extract_text(&[page_number])
            .with_context(|| format!("could not extract text from PDF page {page_number}"))?;
        for (paragraph_index, paragraph) in split_paragraphs(&page_text).into_iter().enumerate() {
            characters = characters.saturating_add(paragraph.chars().count());
            if characters > limits.max_characters {
                bail!(
                    "document exceeds the {} character limit",
                    limits.max_characters
                );
            }
            sections.push(ExtractedSection {
                heading_or_page: if paragraph_index == 0 {
                    format!("Página {page_number}")
                } else {
                    format!("Página {page_number} · párrafo {}", paragraph_index + 1)
                },
                text: paragraph,
            });
        }
    }
    if sections.is_empty() || characters == 0 {
        bail!("PDF has no extractable text layer");
    }
    Ok(ExtractedDocument {
        format: SourceFormat::Pdf,
        sections,
        page_count: pages.len(),
        character_count: characters,
    })
}

fn markdown_heading(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let hashes = trimmed
        .chars()
        .take_while(|character| *character == '#')
        .count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let remainder = trimmed.get(hashes..)?;
    if !remainder.starts_with(char::is_whitespace) {
        return None;
    }
    let title = remainder.trim().trim_end_matches('#').trim();
    (!title.is_empty()).then(|| bounded_heading_or_page(title))
}

fn bounded_heading_or_page(value: &str) -> String {
    value.chars().take(MAX_HEADING_OR_PAGE_CHARS).collect()
}

fn push_section(sections: &mut Vec<ExtractedSection>, heading: &str, text: &str) {
    let text = text.trim();
    if !text.is_empty() {
        sections.push(ExtractedSection {
            heading_or_page: heading.to_owned(),
            text: text.to_owned(),
        });
    }
}

fn split_paragraphs(text: &str) -> Vec<String> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut paragraphs = Vec::new();
    let mut current = Vec::new();
    for line in normalized.lines() {
        let line = line.trim();
        if line.is_empty() {
            if !current.is_empty() {
                paragraphs.push(current.join(" "));
                current.clear();
            }
        } else {
            current.push(line);
        }
    }
    if !current.is_empty() {
        paragraphs.push(current.join(" "));
    }
    paragraphs
        .into_iter()
        .filter(|value| !value.trim().is_empty())
        .collect()
}

fn is_hidden_or_temporary(path: &Path) -> bool {
    for component in path.components() {
        let Component::Normal(name) = component else {
            continue;
        };
        let name = name.to_string_lossy();
        if name.starts_with('.')
            || name.starts_with("~$")
            || name.ends_with('~')
            || name.ends_with(".tmp")
            || name.ends_with(".part")
            || name.ends_with(".swp")
        {
            return true;
        }
    }
    false
}

/// Thin notify adapter. `recv_debounced` coalesces path events for the requested window.
pub struct FolderWatcher {
    _watcher: RecommendedWatcher,
    receiver: Receiver<notify::Result<Event>>,
    debounce: Duration,
}

impl FolderWatcher {
    pub fn new(root: impl AsRef<Path>, debounce: Duration) -> Result<Self> {
        let (sender, receiver) = channel();
        let mut watcher = notify::recommended_watcher(move |event| {
            let _ = sender.send(event);
        })?;
        watcher.watch(root.as_ref(), RecursiveMode::Recursive)?;
        Ok(Self {
            _watcher: watcher,
            receiver,
            debounce,
        })
    }

    pub fn two_second(root: impl AsRef<Path>) -> Result<Self> {
        Self::new(root, Duration::from_secs(2))
    }

    pub fn recv_debounced(&self) -> Result<Vec<PathBuf>> {
        let first = self.receiver.recv().context("folder watcher stopped")??;
        self.coalesce(first)
    }

    /// Waits at most `timeout` for the first filesystem event and then applies
    /// the configured debounce window. This lets long-lived worker loops poll a
    /// cancellation signal instead of blocking forever when a folder is idle.
    pub fn recv_debounced_timeout(&self, timeout: Duration) -> Result<Option<Vec<PathBuf>>> {
        let first = match self.receiver.recv_timeout(timeout) {
            Ok(event) => event?,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return Ok(None),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                bail!("folder watcher stopped")
            }
        };
        self.coalesce(first).map(Some)
    }

    fn coalesce(&self, first: Event) -> Result<Vec<PathBuf>> {
        let started = Instant::now();
        let mut paths: HashMap<PathBuf, ()> =
            first.paths.into_iter().map(|path| (path, ())).collect();
        while let Some(remaining) = self.debounce.checked_sub(started.elapsed()) {
            match self.receiver.recv_timeout(remaining) {
                Ok(Ok(event)) => {
                    paths.extend(event.paths.into_iter().map(|path| (path, ())));
                }
                Ok(Err(error)) => return Err(error.into()),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    bail!("folder watcher stopped")
                }
            }
        }
        let mut paths = paths.into_keys().collect::<Vec<_>>();
        paths.sort();
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_text_pdf(path: &Path, text: &str) {
        use lopdf::content::{Content, Operation};
        use lopdf::{Object, Stream, dictionary};

        let mut document = Document::with_version("1.5");
        let pages_id = document.new_object_id();
        let font_id = document.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Courier",
        });
        let resources_id = document.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 12.into()]),
                Operation::new("Td", vec![72.into(), 720.into()]),
                Operation::new("Tj", vec![Object::string_literal(text)]),
                Operation::new("ET", vec![]),
            ],
        };
        let content_id = document.add_object(Stream::new(
            dictionary! {},
            content.encode().expect("valid PDF content"),
        ));
        let page_id = document.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
        });
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
                "Resources" => resources_id,
                "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            }),
        );
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        document.trailer.set("Root", catalog_id);
        document.compress();
        document.save(path).unwrap();
    }

    #[test]
    fn discovery_accepts_only_supported_visible_regular_files() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("a.md"), "hello").unwrap();
        std::fs::write(temp.path().join("b.markdown"), "hello").unwrap();
        std::fs::write(temp.path().join("ignore.txt"), "hello").unwrap();
        std::fs::write(temp.path().join(".secret.md"), "hello").unwrap();
        std::fs::write(temp.path().join("draft.md~"), "hello").unwrap();
        let files = discover_files(temp.path(), IngestLimits::default()).unwrap();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn discovery_reports_supported_files_over_the_size_limit() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("large.pdf");
        std::fs::write(&path, vec![0_u8; 11]).unwrap();
        let report = discover_files_with_issues(
            temp.path(),
            IngestLimits {
                max_bytes: 10,
                ..IngestLimits::default()
            },
        )
        .unwrap();
        assert!(report.is_complete());
        assert!(report.candidates.is_empty());
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].path, path);
        assert!(report.issues[0].error.contains("10 byte limit"));
        assert!(report.issues[0].error.contains("11 bytes"));
    }

    #[test]
    fn representative_text_is_bounded_utf8_safe_and_covers_both_ends() {
        let sections = (1..=100)
            .map(|page| ExtractedSection {
                heading_or_page: format!("Página {page}"),
                text: format!("MARCADOR-{page} á {}", "contenido ".repeat(8)),
            })
            .collect::<Vec<_>>();
        let document = ExtractedDocument {
            format: SourceFormat::Pdf,
            sections,
            page_count: 100,
            character_count: 9_000,
        };

        let sample = document.representative_text(2_800);
        assert!(sample.len() <= 2_800);
        assert!(sample.is_char_boundary(sample.len()));
        assert!(sample.contains("MARCADOR-1"));
        assert!(sample.contains("MARCADOR-100"));
        assert!(sample.contains("[Fragmento 1/8]"));
        assert!(sample.contains("[Fragmento 8/8]"));
        assert_eq!(sample, document.representative_text(2_800));
    }

    #[test]
    fn representative_text_preserves_a_short_document_verbatim() {
        let document = ExtractedDocument {
            format: SourceFormat::Markdown,
            sections: vec![ExtractedSection {
                heading_or_page: "Título".into(),
                text: "Contenido breve".into(),
            }],
            page_count: 0,
            character_count: 15,
        };
        assert_eq!(document.representative_text(2_800), document.plain_text());
    }

    #[test]
    fn markdown_splits_on_atx_headings() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("runbook.md");
        std::fs::write(
            &path,
            "intro\n\n# Recuperación\nPaso uno.\n## Validación\nPaso dos.",
        )
        .unwrap();
        let extracted =
            extract_file(&path, SourceFormat::Markdown, IngestLimits::default()).unwrap();
        assert_eq!(extracted.sections.len(), 3);
        assert_eq!(extracted.sections[1].heading_or_page, "Recuperación");
        assert_eq!(extracted.page_count, 0);
    }

    #[test]
    fn markdown_bounds_a_near_limit_unicode_heading_without_truncating_body() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("large-heading.md");
        let heading = "á".repeat(1_999_900);
        std::fs::write(&path, format!("# {heading}\ncontenido íntegro")).unwrap();

        let extracted =
            extract_file(&path, SourceFormat::Markdown, IngestLimits::default()).unwrap();

        assert_eq!(extracted.sections.len(), 1);
        assert_eq!(
            extracted.sections[0].heading_or_page.chars().count(),
            MAX_HEADING_OR_PAGE_CHARS
        );
        assert!(
            extracted.sections[0]
                .heading_or_page
                .chars()
                .all(|character| character == 'á')
        );
        assert_eq!(extracted.sections[0].text, "contenido íntegro");

        let chunks = Chunker::default()
            .chunk(&extracted, &WhitespaceTokenizer)
            .unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(
            chunks[0].heading_or_page.chars().count(),
            MAX_HEADING_OR_PAGE_CHARS
        );
        assert_eq!(chunks[0].text, "contenido íntegro");
    }

    #[test]
    fn heading_bound_preserves_short_pdf_page_labels() {
        let label = "Página 500 · párrafo 2";
        assert_eq!(bounded_heading_or_page(label), label);
    }

    #[test]
    fn chunk_overlap_is_exact() {
        let document = ExtractedDocument {
            format: SourceFormat::Markdown,
            sections: vec![ExtractedSection {
                heading_or_page: "H".into(),
                text: (0..10).map(|n| n.to_string()).collect::<Vec<_>>().join(" "),
            }],
            page_count: 0,
            character_count: 19,
        };
        let chunks = Chunker::new(4, 1)
            .unwrap()
            .chunk(&document, &WhitespaceTokenizer)
            .unwrap();
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].text, "0 1 2 3");
        assert_eq!(chunks[1].text, "3 4 5 6");
        assert_eq!(chunks[2].text, "6 7 8 9");
    }

    #[test]
    fn sha_is_stable() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("a.md");
        std::fs::write(&path, "abc").unwrap();
        assert_eq!(
            sha256_file(path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn oversized_markdown_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("a.md");
        std::fs::write(&path, "12345").unwrap();
        let limits = IngestLimits {
            max_bytes: 10,
            max_pdf_pages: 500,
            max_characters: 4,
        };
        assert!(extract_file(path, SourceFormat::Markdown, limits).is_err());
    }

    #[test]
    fn text_layer_pdf_is_split_by_page() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("text.pdf");
        write_text_pdf(&path, "Payment recovery procedure");
        let extracted = extract_file(path, SourceFormat::Pdf, IngestLimits::default()).unwrap();
        assert_eq!(extracted.page_count, 1);
        assert_eq!(extracted.sections[0].heading_or_page, "Página 1");
        assert!(extracted.sections[0].text.contains("Payment recovery"));
    }

    #[test]
    fn invalid_pdf_is_rejected_without_panicking() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("broken.pdf");
        std::fs::write(&path, b"not a PDF").unwrap();
        assert!(extract_file(path, SourceFormat::Pdf, IngestLimits::default()).is_err());
    }

    #[test]
    fn pdf_without_a_text_layer_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("empty.pdf");
        write_text_pdf(&path, "");
        let error = extract_file(path, SourceFormat::Pdf, IngestLimits::default()).unwrap_err();
        assert!(error.to_string().contains("no extractable text layer"));
    }

    #[test]
    fn encrypted_pdf_is_rejected_even_with_an_empty_password() {
        use lopdf::{EncryptionState, EncryptionVersion, Object, Permissions, StringFormat};

        let temp = tempfile::tempdir().unwrap();
        let plain = temp.path().join("plain.pdf");
        let encrypted = temp.path().join("encrypted.pdf");
        write_text_pdf(&plain, "confidential text");
        let mut document = Document::load(plain).unwrap();
        document.trailer.set(
            "ID",
            Object::Array(vec![
                Object::String(vec![1_u8; 16], StringFormat::Literal),
                Object::String(vec![2_u8; 16], StringFormat::Literal),
            ]),
        );
        let state = EncryptionState::try_from(EncryptionVersion::V2 {
            document: &document,
            owner_password: "",
            user_password: "",
            key_length: 128,
            permissions: Permissions::all(),
        })
        .unwrap();
        document.encrypt(&state).unwrap();
        document.save(&encrypted).unwrap();

        let error =
            extract_file(encrypted, SourceFormat::Pdf, IngestLimits::default()).unwrap_err();
        assert!(error.to_string().contains("encrypted PDFs"));
    }

    #[test]
    fn watcher_timeout_returns_without_an_event() {
        let temp = tempfile::tempdir().unwrap();
        let watcher = FolderWatcher::new(temp.path(), Duration::from_millis(10)).unwrap();

        assert!(
            watcher
                .recv_debounced_timeout(Duration::from_millis(25))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn watcher_can_receive_after_a_timeout() {
        let temp = tempfile::tempdir().unwrap();
        let watcher = FolderWatcher::new(temp.path(), Duration::from_millis(20)).unwrap();
        assert!(
            watcher
                .recv_debounced_timeout(Duration::from_millis(10))
                .unwrap()
                .is_none()
        );

        let changed = temp.path().join("changed.md");
        std::fs::write(&changed, "content").unwrap();
        let paths = watcher
            .recv_debounced_timeout(Duration::from_secs(2))
            .unwrap()
            .expect("the write must produce a watcher event");

        // Some notify backends report the changed file while others report its
        // watched parent directory. Either proves the receiver remained usable.
        assert!(!paths.is_empty());
    }
}
