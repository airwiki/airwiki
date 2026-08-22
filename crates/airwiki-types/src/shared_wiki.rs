//! Bounded frames used to browse an explicitly granted, published OKF Wiki over
//! the LAN. Version 2 carries the complete read-only workspace while retaining
//! the summary-only version 1 contract for compatibility.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    ConceptAssurance, ConceptType, MAX_SNIPPET_CHARS, OkfCompatibility,
    SHARED_WIKI_BROWSE_PROTOCOL, SHARED_WIKI_BROWSE_PROTOCOL_V2,
};

/// Maximum concepts returned by one authenticated LAN browse request.
pub const MAX_SHARED_WIKI_PAGE_SIZE: u8 = 50;
/// Maximum graph edges returned in one remote workspace page.
pub const MAX_SHARED_WIKI_GRAPH_PAGE_SIZE: usize = 256;
/// Maximum UTF-8 Markdown body returned for one published OKF page.
pub const MAX_SHARED_WIKI_DOCUMENT_BYTES: usize = 1024 * 1024;
/// Transport ceiling for one explicit remote Wiki document response.
///
/// A published page remains capped at one MiB. The larger wire ceiling covers
/// its bounded metadata, concept descriptors, graph edges and encoding
/// overhead, so every valid page can be transferred without silent truncation.
pub const MAX_SHARED_WIKI_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

/// A page inside the published OKF bundle. These identifiers never contain a
/// local filesystem path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PublishedWikiPageId {
    Index,
    Log,
    Concept { concept_id: Uuid },
}

/// Requests one complete, published OKF Markdown page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedWikiPageRequest {
    pub page: PublishedWikiPageId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_fingerprint: Option<String>,
}

/// A safe logical entry in the published bundle hierarchy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedWikiPageDescriptor {
    pub page: PublishedWikiPageId,
    pub logical_path: String,
    pub title: String,
    pub fingerprint: String,
}

/// One verified internal relation between published OKF pages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedWikiGraphLink {
    pub source: PublishedWikiPageId,
    pub target: PublishedWikiPageId,
    pub label: String,
}

/// Progressive structure for a remote Wiki. Document descriptors follow the
/// same order as the enclosing concept summaries; graph edges have an
/// independent numeric cursor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedWikiWorkspacePage {
    /// Fingerprint of the complete published bundle generation represented by
    /// this bounded response. Consumers must not merge pages with different
    /// fingerprints.
    pub workspace_fingerprint: String,
    pub reserved_pages: Vec<PublishedWikiPageDescriptor>,
    pub documents: Vec<PublishedWikiPageDescriptor>,
    pub links: Vec<PublishedWikiGraphLink>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_graph_cursor: Option<u32>,
}

/// Complete Markdown and frontmatter projection for one published OKF page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedWikiDocument {
    pub descriptor: PublishedWikiPageDescriptor,
    pub body_markdown: String,
    pub metadata: Vec<(String, String)>,
    pub backlinks: Vec<PublishedWikiPageId>,
    pub truncated: bool,
}

/// Requests one bounded page from a Wiki already granted to the authenticated peer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedWikiBrowseRequest {
    pub protocol_version: String,
    pub request_id: Uuid,
    pub collection_id: Uuid,
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_concept_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_cursor: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<PublishedWikiPageRequest>,
    pub limit: u8,
}

impl SharedWikiBrowseRequest {
    pub fn new(collection_id: Uuid, cursor: Option<String>, limit: u8) -> Self {
        Self {
            protocol_version: SHARED_WIKI_BROWSE_PROTOCOL_V2.to_owned(),
            request_id: Uuid::new_v4(),
            collection_id,
            cursor,
            target_concept_id: None,
            graph_cursor: None,
            page: None,
            limit,
        }
    }

    pub fn from_concept(collection_id: Uuid, target_concept_id: Uuid, limit: u8) -> Self {
        let mut request = Self::new(collection_id, None, limit);
        request.target_concept_id = Some(target_concept_id);
        request
    }

    pub fn validate(&self) -> Result<(), SharedWikiContractError> {
        if !matches!(
            self.protocol_version.as_str(),
            SHARED_WIKI_BROWSE_PROTOCOL | SHARED_WIKI_BROWSE_PROTOCOL_V2
        ) {
            return Err(SharedWikiContractError::UnsupportedProtocol);
        }
        if !(1..=MAX_SHARED_WIKI_PAGE_SIZE).contains(&self.limit)
            || (self.cursor.is_some() && self.target_concept_id.is_some())
            || (self.protocol_version == SHARED_WIKI_BROWSE_PROTOCOL
                && (self.graph_cursor.is_some() || self.page.is_some()))
            || self
                .cursor
                .as_deref()
                .is_some_and(|cursor| Uuid::parse_str(cursor).is_err())
        {
            return Err(SharedWikiContractError::InvalidPage);
        }
        if let Some(page) = &self.page {
            validate_page_request(page)?;
            if let PublishedWikiPageId::Concept { concept_id } = page.page
                && self
                    .target_concept_id
                    .is_some_and(|target| target != concept_id)
            {
                return Err(SharedWikiContractError::InvalidPage);
            }
        }
        Ok(())
    }

    /// Adapts a current request to the protocol negotiated by libp2p.
    pub fn prepare_for_protocol(&mut self, protocol: &str) -> Result<(), SharedWikiContractError> {
        match protocol {
            SHARED_WIKI_BROWSE_PROTOCOL_V2 => {}
            SHARED_WIKI_BROWSE_PROTOCOL => {
                if self.target_concept_id.is_none()
                    && let Some(PublishedWikiPageRequest {
                        page: PublishedWikiPageId::Concept { concept_id },
                        ..
                    }) = self.page.as_ref()
                {
                    self.target_concept_id = Some(*concept_id);
                }
                self.graph_cursor = None;
                self.page = None;
            }
            _ => return Err(SharedWikiContractError::UnsupportedProtocol),
        }
        self.protocol_version = protocol.to_owned();
        Ok(())
    }
}

/// Non-sensitive Wiki metadata disclosed only after trust and grant revalidation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedWikiDescriptor {
    pub collection_id: Uuid,
    pub name: String,
    pub okf_compatibility: OkfCompatibility,
}

/// Read-only concept metadata used for the file outline. Version 2 may pair it
/// with the complete published OKF page; original source paths never cross this
/// contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SharedWikiConceptSummary {
    pub concept_id: Uuid,
    pub concept_type: ConceptType,
    pub title: String,
    pub description: String,
    pub language: String,
    pub tags: Vec<String>,
    pub summary: String,
    pub logical_resource_uri: String,
    pub source_revision: u32,
    pub updated_at: DateTime<Utc>,
    pub lifecycle_status: Option<String>,
    pub assurance: Option<ConceptAssurance>,
}

/// One authenticated, bounded page from a Wiki shared with the requesting device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SharedWikiBrowsePage {
    pub protocol_version: String,
    pub request_id: Uuid,
    pub wiki: SharedWikiDescriptor,
    pub concepts: Vec<SharedWikiConceptSummary>,
    pub next_cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<PublishedWikiWorkspacePage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<PublishedWikiDocument>,
}

impl SharedWikiBrowsePage {
    pub fn validate_for(
        &self,
        request: &SharedWikiBrowseRequest,
    ) -> Result<(), SharedWikiContractError> {
        let requested_after = request
            .cursor
            .as_deref()
            .map(Uuid::parse_str)
            .transpose()
            .map_err(|_| SharedWikiContractError::InvalidPage)?;
        let next_cursor = self
            .next_cursor
            .as_deref()
            .map(Uuid::parse_str)
            .transpose()
            .map_err(|_| SharedWikiContractError::InvalidPage)?;
        if self.protocol_version != request.protocol_version
            || self.request_id != request.request_id
            || self.wiki.collection_id != request.collection_id
            || !self.wiki.okf_compatibility.permits_external_disclosure()
            || self.concepts.len() > usize::from(request.limit)
            || (request.page.is_none()
                && request.target_concept_id.is_some_and(|target| {
                    self.concepts.first().map(|concept| concept.concept_id) != Some(target)
                }))
        {
            return Err(SharedWikiContractError::InvalidPage);
        }
        validate_text(&self.wiki.name, 240)?;
        let mut previous_concept = requested_after;
        for concept in &self.concepts {
            if previous_concept.is_some_and(|previous| concept.concept_id <= previous)
                || concept.lifecycle_status.as_deref() != Some("stable")
            {
                return Err(SharedWikiContractError::InvalidPage);
            }
            previous_concept = Some(concept.concept_id);
            validate_text(&concept.concept_type.to_string(), 120)?;
            validate_text(&concept.title, 240)?;
            validate_optional_text(&concept.description, 1_000)?;
            validate_text(&concept.language, 16)?;
            if concept.tags.len() > 64
                || concept.summary.chars().count() > MAX_SNIPPET_CHARS
                || concept.summary.chars().any(char::is_control)
            {
                return Err(SharedWikiContractError::InvalidPage);
            }
            validate_text(&concept.logical_resource_uri, 2_048)?;
            for tag in &concept.tags {
                validate_text(tag, 64)?;
            }
        }
        if next_cursor.is_some()
            && (self.concepts.is_empty()
                || next_cursor != self.concepts.last().map(|concept| concept.concept_id))
        {
            return Err(SharedWikiContractError::InvalidPage);
        }
        validate_workspace_response(
            request.protocol_version == SHARED_WIKI_BROWSE_PROTOCOL_V2,
            request.graph_cursor,
            request.page.as_ref(),
            &self.concepts,
            self.workspace.as_ref(),
            self.document.as_ref(),
        )?;
        Ok(())
    }

    /// Removes v2-only content when an older peer negotiated the summary-only
    /// contract.
    pub fn prepare_for_protocol(&mut self, protocol: &str) -> Result<(), SharedWikiContractError> {
        match protocol {
            SHARED_WIKI_BROWSE_PROTOCOL_V2 => {}
            SHARED_WIKI_BROWSE_PROTOCOL => {
                self.workspace = None;
                self.document = None;
            }
            _ => return Err(SharedWikiContractError::UnsupportedProtocol),
        }
        self.protocol_version = protocol.to_owned();
        Ok(())
    }
}

fn validate_page_request(
    request: &PublishedWikiPageRequest,
) -> Result<(), SharedWikiContractError> {
    if request
        .expected_fingerprint
        .as_deref()
        .is_some_and(|fingerprint| !valid_fingerprint(fingerprint))
    {
        return Err(SharedWikiContractError::InvalidPage);
    }
    Ok(())
}

pub(crate) fn validate_workspace_response<Concept>(
    full_workspace_protocol: bool,
    requested_graph_cursor: Option<u32>,
    requested_page: Option<&PublishedWikiPageRequest>,
    concepts: &[Concept],
    workspace: Option<&PublishedWikiWorkspacePage>,
    document: Option<&PublishedWikiDocument>,
) -> Result<(), SharedWikiContractError>
where
    Concept: PublishedConceptId,
{
    if !full_workspace_protocol {
        return if workspace.is_none() && document.is_none() {
            Ok(())
        } else {
            Err(SharedWikiContractError::InvalidPage)
        };
    }
    let workspace = workspace.ok_or(SharedWikiContractError::InvalidPage)?;
    if !valid_fingerprint(&workspace.workspace_fingerprint)
        || workspace.documents.len() != concepts.len()
        || workspace.reserved_pages.len() > 2
        || workspace.links.len() > MAX_SHARED_WIKI_GRAPH_PAGE_SIZE
        || workspace
            .documents
            .iter()
            .zip(concepts)
            .any(|(descriptor, concept)| {
                descriptor.page
                    != (PublishedWikiPageId::Concept {
                        concept_id: concept.published_concept_id(),
                    })
                    || validate_descriptor(descriptor).is_err()
            })
        || workspace.reserved_pages.iter().any(|descriptor| {
            validate_descriptor(descriptor).is_err()
                || !matches!(
                    (&descriptor.page, descriptor.logical_path.as_str()),
                    (PublishedWikiPageId::Index, "index.md") | (PublishedWikiPageId::Log, "log.md")
                )
        })
        || workspace
            .links
            .iter()
            .any(|link| validate_graph_link(link).is_err())
    {
        return Err(SharedWikiContractError::InvalidPage);
    }
    let descriptors = workspace
        .reserved_pages
        .iter()
        .chain(workspace.documents.iter())
        .collect::<Vec<_>>();
    if descriptors.iter().enumerate().any(|(index, descriptor)| {
        descriptors
            .iter()
            .skip(index.saturating_add(1))
            .any(|candidate| {
                descriptor.page == candidate.page
                    || descriptor.logical_path == candidate.logical_path
            })
    }) {
        return Err(SharedWikiContractError::InvalidPage);
    }
    if workspace.links.iter().enumerate().any(|(index, link)| {
        workspace
            .links
            .iter()
            .skip(index.saturating_add(1))
            .any(|candidate| link == candidate)
    }) {
        return Err(SharedWikiContractError::InvalidPage);
    }
    match requested_graph_cursor {
        None => {
            if !workspace.links.is_empty() || workspace.next_graph_cursor.is_some() {
                return Err(SharedWikiContractError::InvalidPage);
            }
        }
        Some(cursor) => {
            if let Some(next_cursor) = workspace.next_graph_cursor {
                let link_count = u32::try_from(workspace.links.len())
                    .map_err(|_| SharedWikiContractError::InvalidPage)?;
                if workspace.links.is_empty() || cursor.checked_add(link_count) != Some(next_cursor)
                {
                    return Err(SharedWikiContractError::InvalidPage);
                }
            }
        }
    }
    if let Some(document) = document {
        validate_document(document)?;
        if requested_page.is_none_or(|page| {
            page.page != document.descriptor.page
                || page
                    .expected_fingerprint
                    .as_deref()
                    .is_some_and(|expected| document.descriptor.fingerprint.as_str() != expected)
        }) {
            return Err(SharedWikiContractError::InvalidPage);
        }
    } else if requested_page.is_some() {
        return Err(SharedWikiContractError::InvalidPage);
    }
    Ok(())
}

pub(crate) trait PublishedConceptId {
    fn published_concept_id(&self) -> Uuid;
}

impl PublishedConceptId for SharedWikiConceptSummary {
    fn published_concept_id(&self) -> Uuid {
        self.concept_id
    }
}

fn validate_descriptor(
    descriptor: &PublishedWikiPageDescriptor,
) -> Result<(), SharedWikiContractError> {
    validate_logical_path(&descriptor.logical_path)?;
    validate_text(&descriptor.title, 240)?;
    if !valid_fingerprint(&descriptor.fingerprint) {
        return Err(SharedWikiContractError::InvalidPage);
    }
    Ok(())
}

fn validate_graph_link(link: &PublishedWikiGraphLink) -> Result<(), SharedWikiContractError> {
    validate_optional_text(&link.label, 300)
}

fn validate_document(document: &PublishedWikiDocument) -> Result<(), SharedWikiContractError> {
    validate_descriptor(&document.descriptor)?;
    if document.truncated
        || document.body_markdown.len() > MAX_SHARED_WIKI_DOCUMENT_BYTES
        || document.metadata.len() > 256
        || document.backlinks.len() > 5_000
        || document
            .metadata
            .iter()
            .any(|(key, value)| key.len() > 240 || value.len() > 16 * 1024)
    {
        return Err(SharedWikiContractError::InvalidPage);
    }
    Ok(())
}

fn validate_logical_path(path: &str) -> Result<(), SharedWikiContractError> {
    if path.is_empty()
        || path.len() > 2_048
        || path.starts_with('/')
        || path.contains('\\')
        || path.chars().any(char::is_control)
        || path
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        return Err(SharedWikiContractError::InvalidPage);
    }
    Ok(())
}

fn valid_fingerprint(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SharedWikiContractError {
    #[error("shared Wiki protocol is unsupported")]
    UnsupportedProtocol,
    #[error("shared Wiki page is invalid")]
    InvalidPage,
}

fn validate_text(value: &str, maximum_chars: usize) -> Result<(), SharedWikiContractError> {
    if value.trim().is_empty()
        || value.chars().count() > maximum_chars
        || value.chars().any(char::is_control)
    {
        return Err(SharedWikiContractError::InvalidPage);
    }
    Ok(())
}

fn validate_optional_text(
    value: &str,
    maximum_chars: usize,
) -> Result<(), SharedWikiContractError> {
    if value.chars().count() > maximum_chars || value.chars().any(char::is_control) {
        return Err(SharedWikiContractError::InvalidPage);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(request: &SharedWikiBrowseRequest) -> SharedWikiBrowsePage {
        let concept = SharedWikiConceptSummary {
            concept_id: Uuid::new_v4(),
            concept_type: ConceptType::Other("Guide".to_owned()),
            title: "Primeros pasos".to_owned(),
            description: String::new(),
            language: "es".to_owned(),
            tags: Vec::new(),
            summary: "Resumen compartido y acotado.".to_owned(),
            logical_resource_uri: "urn:airwiki:shared:concept".to_owned(),
            source_revision: 1,
            updated_at: Utc::now(),
            lifecycle_status: Some("stable".to_owned()),
            assurance: None,
        };
        let workspace = (request.protocol_version == SHARED_WIKI_BROWSE_PROTOCOL_V2).then(|| {
            PublishedWikiWorkspacePage {
                workspace_fingerprint: "b".repeat(64),
                reserved_pages: Vec::new(),
                documents: vec![PublishedWikiPageDescriptor {
                    page: PublishedWikiPageId::Concept {
                        concept_id: concept.concept_id,
                    },
                    logical_path: format!("concepts/{}.md", concept.concept_id),
                    title: concept.title.clone(),
                    fingerprint: "a".repeat(64),
                }],
                links: Vec::new(),
                next_graph_cursor: None,
            }
        });
        SharedWikiBrowsePage {
            protocol_version: request.protocol_version.clone(),
            request_id: request.request_id,
            wiki: SharedWikiDescriptor {
                collection_id: request.collection_id,
                name: "Atlas compartido".to_owned(),
                okf_compatibility: OkfCompatibility::DeclaredV02,
            },
            concepts: vec![concept],
            next_cursor: None,
            workspace,
            document: None,
        }
    }

    #[test]
    fn validates_a_bounded_page_for_the_requested_wiki() {
        let request = SharedWikiBrowseRequest::new(Uuid::new_v4(), None, 50);
        assert!(request.validate().is_ok());
        assert!(page(&request).validate_for(&request).is_ok());
    }

    #[test]
    fn rejects_a_page_for_a_different_wiki() {
        let request = SharedWikiBrowseRequest::new(Uuid::new_v4(), None, 50);
        let mut response = page(&request);
        response.wiki.collection_id = Uuid::new_v4();
        assert_eq!(
            response.validate_for(&request),
            Err(SharedWikiContractError::InvalidPage)
        );
    }

    #[test]
    fn rejects_a_page_whose_okf_compatibility_forbids_disclosure() {
        let request = SharedWikiBrowseRequest::new(Uuid::new_v4(), None, 50);
        let mut response = page(&request);
        response.wiki.okf_compatibility = OkfCompatibility::LegacyV01;
        assert_eq!(
            response.validate_for(&request),
            Err(SharedWikiContractError::InvalidPage)
        );

        response.wiki.okf_compatibility = OkfCompatibility::FutureRestricted {
            declared_version: "0.3".to_owned(),
        };
        assert_eq!(
            response.validate_for(&request),
            Err(SharedWikiContractError::InvalidPage)
        );
    }

    #[test]
    fn anchored_page_must_start_with_the_requested_concept() {
        let target = Uuid::new_v4();
        let request = SharedWikiBrowseRequest::from_concept(Uuid::new_v4(), target, 50);
        let mut response = page(&request);
        assert_eq!(
            response.validate_for(&request),
            Err(SharedWikiContractError::InvalidPage)
        );

        response.concepts[0].concept_id = target;
        response.workspace.as_mut().unwrap().documents[0].page =
            PublishedWikiPageId::Concept { concept_id: target };
        assert!(response.validate_for(&request).is_ok());
    }

    #[test]
    fn rejects_a_cursor_combined_with_a_target() {
        let mut request = SharedWikiBrowseRequest::from_concept(
            Uuid::new_v4(),
            Uuid::new_v4(),
            MAX_SHARED_WIKI_PAGE_SIZE,
        );
        request.cursor = Some(Uuid::new_v4().to_string());
        assert_eq!(
            request.validate(),
            Err(SharedWikiContractError::InvalidPage)
        );
    }

    #[test]
    fn rejects_invalid_or_non_monotonic_pagination() {
        let collection_id = Uuid::new_v4();
        let invalid_cursor =
            SharedWikiBrowseRequest::new(collection_id, Some("not-a-concept-id".to_owned()), 50);
        assert_eq!(
            invalid_cursor.validate(),
            Err(SharedWikiContractError::InvalidPage)
        );

        let first_id = Uuid::from_u128(2);
        let request =
            SharedWikiBrowseRequest::new(collection_id, Some(Uuid::from_u128(1).to_string()), 2);
        let mut response = page(&request);
        response.concepts[0].concept_id = first_id;
        response.concepts.push(response.concepts[0].clone());
        response.next_cursor = Some(first_id.to_string());
        assert_eq!(
            response.validate_for(&request),
            Err(SharedWikiContractError::InvalidPage)
        );
    }

    #[test]
    fn rejects_truncated_published_pages() {
        let mut request = SharedWikiBrowseRequest::new(Uuid::new_v4(), None, 1);
        let mut response = page(&request);
        let descriptor = response
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.documents.first())
            .cloned()
            .unwrap();
        request.page = Some(PublishedWikiPageRequest {
            page: descriptor.page,
            expected_fingerprint: Some(descriptor.fingerprint.clone()),
        });
        response.document = Some(PublishedWikiDocument {
            descriptor,
            body_markdown: "Complete published page".to_owned(),
            metadata: Vec::new(),
            backlinks: Vec::new(),
            truncated: false,
        });
        assert!(response.validate_for(&request).is_ok());

        response.document.as_mut().unwrap().truncated = true;
        assert_eq!(
            response.validate_for(&request),
            Err(SharedWikiContractError::InvalidPage)
        );
    }

    #[test]
    fn rejects_a_document_that_does_not_match_the_requested_fingerprint() {
        let mut request = SharedWikiBrowseRequest::new(Uuid::new_v4(), None, 1);
        let mut response = page(&request);
        let descriptor = response
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.documents.first())
            .cloned()
            .unwrap();
        request.page = Some(PublishedWikiPageRequest {
            page: descriptor.page,
            expected_fingerprint: Some("c".repeat(64)),
        });
        response.document = Some(PublishedWikiDocument {
            descriptor,
            body_markdown: "Stale published page".to_owned(),
            metadata: Vec::new(),
            backlinks: Vec::new(),
            truncated: false,
        });

        assert_eq!(
            response.validate_for(&request),
            Err(SharedWikiContractError::InvalidPage)
        );
    }

    #[test]
    fn graph_cursor_must_advance_by_the_returned_edges() {
        let mut request = SharedWikiBrowseRequest::new(Uuid::new_v4(), None, 1);
        request.graph_cursor = Some(0);
        let mut response = page(&request);
        let concept_page = response
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.documents.first())
            .map(|descriptor| descriptor.page)
            .unwrap();
        let workspace = response.workspace.as_mut().unwrap();
        workspace.links.push(PublishedWikiGraphLink {
            source: concept_page,
            target: concept_page,
            label: "Related".to_owned(),
        });
        workspace.next_graph_cursor = Some(1);
        assert!(response.validate_for(&request).is_ok());

        response.workspace.as_mut().unwrap().next_graph_cursor = Some(0);
        assert_eq!(
            response.validate_for(&request),
            Err(SharedWikiContractError::InvalidPage)
        );
    }

    #[test]
    fn rejects_invalid_workspace_fingerprints_and_duplicate_graph_edges() {
        let mut request = SharedWikiBrowseRequest::new(Uuid::new_v4(), None, 1);
        request.graph_cursor = Some(0);
        let mut response = page(&request);
        response.workspace.as_mut().unwrap().workspace_fingerprint = "invalid".to_owned();
        assert_eq!(
            response.validate_for(&request),
            Err(SharedWikiContractError::InvalidPage)
        );

        let workspace = response.workspace.as_mut().unwrap();
        workspace.workspace_fingerprint = "b".repeat(64);
        let concept_page = workspace.documents[0].page;
        let link = PublishedWikiGraphLink {
            source: concept_page,
            target: concept_page,
            label: "Related".to_owned(),
        };
        workspace.links = vec![link.clone(), link];
        assert_eq!(
            response.validate_for(&request),
            Err(SharedWikiContractError::InvalidPage)
        );
    }
}
