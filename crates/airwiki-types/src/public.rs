use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    ConceptType, MAX_QUERY_BYTES, MAX_SNIPPET_CHARS, MAX_TOP_K, MIN_TOP_K, OkfCompatibility,
    PUBLIC_BROWSE_PROTOCOL, PUBLIC_BROWSE_PROTOCOL_V2, PUBLIC_BROWSE_PROTOCOL_V3,
    PUBLIC_BROWSE_PROTOCOL_V4, PUBLIC_CATALOG_PROTOCOL, PUBLIC_CATALOG_PROTOCOL_V2,
    PUBLIC_SEARCH_PROTOCOL, PUBLIC_SEARCH_PROTOCOL_V2, PublishedConceptId, PublishedWikiDocument,
    PublishedWikiPageId, PublishedWikiPageRequest, PublishedWikiWorkspacePage, SearchPurpose,
    SearchResponse,
};

pub const MAX_PUBLIC_PAGE_SIZE: u8 = 50;
pub const MAX_PUBLIC_CANDIDATES: u8 = 64;
pub const MAX_PUBLIC_ROUTING_TERMS: usize = 256;
pub const MAX_PUBLIC_ROUTES: usize = 8;
pub const MAX_PUBLIC_MANIFEST_LIFETIME_SECONDS: i64 = 24 * 60 * 60;
const MAX_PUBLIC_MANIFEST_FUTURE_SKEW_SECONDS: i64 = 5 * 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicCollectionManifest {
    pub protocol_version: String,
    pub publisher_id: String,
    pub collection_id: Uuid,
    pub sequence: u64,
    pub publication_fingerprint: String,
    pub name: String,
    pub description: String,
    pub languages: Vec<String>,
    pub concept_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub okf_compatibility: Option<OkfCompatibility>,
    pub routing_terms: Vec<String>,
    pub routes: Vec<String>,
    pub updated_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl PublicCollectionManifest {
    pub fn validate(&self, now: DateTime<Utc>) -> Result<(), PublicContractError> {
        if !protocol_is_supported(
            &self.protocol_version,
            PUBLIC_CATALOG_PROTOCOL,
            PUBLIC_CATALOG_PROTOCOL_V2,
        ) {
            return Err(PublicContractError::UnsupportedProtocol);
        }
        validate_text(&self.publisher_id, 128)?;
        validate_text(&self.name, 240)?;
        validate_optional_text(&self.description, 1_000)?;
        if self.publication_fingerprint.len() != 64
            || !self
                .publication_fingerprint
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(PublicContractError::InvalidFingerprint);
        }
        if self.languages.len() > 16
            || self.routing_terms.len() > MAX_PUBLIC_ROUTING_TERMS
            || self.routes.len() > MAX_PUBLIC_ROUTES
        {
            return Err(PublicContractError::TooManyItems);
        }
        for language in &self.languages {
            validate_text(language, 16)?;
        }
        for term in &self.routing_terms {
            validate_text(term, 64)?;
        }
        for route in &self.routes {
            validate_text(route, 500)?;
        }
        if self.updated_at > self.expires_at || self.expires_at <= now {
            return Err(PublicContractError::Expired);
        }
        if self.expires_at > now + chrono::Duration::seconds(MAX_PUBLIC_MANIFEST_LIFETIME_SECONDS) {
            return Err(PublicContractError::InvalidExpiry);
        }
        if self.updated_at
            > now + chrono::Duration::seconds(MAX_PUBLIC_MANIFEST_FUTURE_SKEW_SECONDS)
            || self.expires_at.signed_duration_since(self.updated_at)
                > chrono::Duration::seconds(MAX_PUBLIC_MANIFEST_LIFETIME_SECONDS)
        {
            return Err(PublicContractError::InvalidExpiry);
        }
        Ok(())
    }

    pub fn summary(&self) -> PublicCollectionSummary {
        PublicCollectionSummary {
            publisher_id: self.publisher_id.clone(),
            collection_id: self.collection_id,
            manifest_sequence: self.sequence,
            name: self.name.clone(),
            description: self.description.clone(),
            languages: self.languages.clone(),
            concept_count: self.concept_count,
            okf_compatibility: self.okf_compatibility.clone(),
            updated_at: self.updated_at,
            expires_at: self.expires_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedPublicCollectionManifest {
    pub manifest: PublicCollectionManifest,
    pub public_key: Vec<u8>,
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicCollectionTombstone {
    pub protocol_version: String,
    pub publisher_id: String,
    pub collection_id: Uuid,
    pub sequence: u64,
    pub withdrawn_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedPublicCollectionTombstone {
    pub tombstone: PublicCollectionTombstone,
    pub public_key: Vec<u8>,
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicCollectionSummary {
    pub publisher_id: String,
    pub collection_id: Uuid,
    pub manifest_sequence: u64,
    pub name: String,
    pub description: String,
    pub languages: Vec<String>,
    pub concept_count: u32,
    pub okf_compatibility: Option<OkfCompatibility>,
    pub updated_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PublicConceptSummary {
    pub publisher_id: String,
    pub collection_id: Uuid,
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
    #[serde(default)]
    pub lifecycle_status: Option<String>,
    #[serde(default)]
    pub assurance: Option<crate::ConceptAssurance>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicCatalogQuery {
    pub protocol_version: String,
    pub request_id: Uuid,
    pub query: String,
    pub languages: Vec<String>,
    pub limit: u8,
}

impl PublicCatalogQuery {
    pub fn validate(&self) -> Result<(), PublicContractError> {
        if !protocol_is_supported(
            &self.protocol_version,
            PUBLIC_CATALOG_PROTOCOL,
            PUBLIC_CATALOG_PROTOCOL_V2,
        ) {
            return Err(PublicContractError::UnsupportedProtocol);
        }
        validate_text(&self.query, MAX_QUERY_BYTES)?;
        if self.languages.len() > 8 || !(1..=MAX_PUBLIC_CANDIDATES).contains(&self.limit) {
            return Err(PublicContractError::InvalidLimit);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicCatalogResponse {
    pub request_id: Uuid,
    pub collections: Vec<SignedPublicCollectionManifest>,
    pub partial: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicSearchRequest {
    pub protocol_version: String,
    pub request_id: Uuid,
    pub query: String,
    pub purpose: SearchPurpose,
    pub collections: Vec<PublicCollectionTarget>,
    pub top_k: u8,
}

impl PublicSearchRequest {
    pub fn validate(&self) -> Result<(), PublicContractError> {
        if !protocol_is_supported(
            &self.protocol_version,
            PUBLIC_SEARCH_PROTOCOL,
            PUBLIC_SEARCH_PROTOCOL_V2,
        ) {
            return Err(PublicContractError::UnsupportedProtocol);
        }
        validate_text(&self.query, MAX_QUERY_BYTES)?;
        if self.collections.is_empty()
            || self.collections.len() > 2
            || !(MIN_TOP_K..=MAX_TOP_K).contains(&self.top_k)
        {
            return Err(PublicContractError::InvalidLimit);
        }
        if self.collections.iter().any(|collection| {
            collection.publication_fingerprint.len() != 64
                || !collection
                    .publication_fingerprint
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
        }) {
            return Err(PublicContractError::InvalidFingerprint);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicCollectionTarget {
    pub collection_id: Uuid,
    pub manifest_sequence: u64,
    pub publication_fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicSearchResponse {
    pub protocol_version: String,
    pub manifest_sequences: Vec<PublicCollectionRevision>,
    pub response: SearchResponse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicCollectionRevision {
    pub collection_id: Uuid,
    pub manifest_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicBrowseRequest {
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

impl PublicBrowseRequest {
    /// Adapts the anchored v3 request to the cursor-only v1/v2 contract.
    ///
    /// Concept UUIDs are stored and paged in canonical lexical order. The
    /// immediately preceding UUID therefore makes the selected concept the
    /// first item returned by an older AirWiki peer without changing either
    /// legacy wire shape.
    pub fn prepare_for_protocol(&mut self, protocol: &str) -> Result<(), PublicContractError> {
        if self.cursor.is_some() && self.target_concept_id.is_some() {
            return Err(PublicContractError::InvalidLimit);
        }
        match protocol {
            PUBLIC_BROWSE_PROTOCOL_V4 => {}
            PUBLIC_BROWSE_PROTOCOL_V3 => {
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
            PUBLIC_BROWSE_PROTOCOL | PUBLIC_BROWSE_PROTOCOL_V2 => {
                if self.target_concept_id.is_none()
                    && let Some(PublishedWikiPageRequest {
                        page: PublishedWikiPageId::Concept { concept_id },
                        ..
                    }) = self.page.as_ref()
                {
                    self.target_concept_id = Some(*concept_id);
                }
                if let Some(target) = self.target_concept_id.take() {
                    self.cursor = target
                        .as_u128()
                        .checked_sub(1)
                        .map(Uuid::from_u128)
                        .map(|cursor| cursor.to_string());
                }
                self.graph_cursor = None;
                self.page = None;
            }
            _ => return Err(PublicContractError::UnsupportedProtocol),
        }
        self.protocol_version = protocol.to_owned();
        Ok(())
    }

    pub fn validate(&self) -> Result<(), PublicContractError> {
        if !matches!(
            self.protocol_version.as_str(),
            PUBLIC_BROWSE_PROTOCOL
                | PUBLIC_BROWSE_PROTOCOL_V2
                | PUBLIC_BROWSE_PROTOCOL_V3
                | PUBLIC_BROWSE_PROTOCOL_V4
        ) {
            return Err(PublicContractError::UnsupportedProtocol);
        }
        if !(1..=MAX_PUBLIC_PAGE_SIZE).contains(&self.limit)
            || (self.cursor.is_some() && self.target_concept_id.is_some())
            || (self.target_concept_id.is_some()
                && !matches!(
                    self.protocol_version.as_str(),
                    PUBLIC_BROWSE_PROTOCOL_V3 | PUBLIC_BROWSE_PROTOCOL_V4
                ))
            || (self.protocol_version != PUBLIC_BROWSE_PROTOCOL_V4
                && (self.graph_cursor.is_some() || self.page.is_some()))
            || self
                .cursor
                .as_ref()
                .is_some_and(|cursor| Uuid::parse_str(cursor).is_err())
        {
            return Err(PublicContractError::InvalidLimit);
        }
        if let Some(page) = &self.page
            && (page
                .expected_fingerprint
                .as_deref()
                .is_some_and(|fingerprint| !valid_fingerprint(fingerprint))
                || matches!(page.page, PublishedWikiPageId::Concept { concept_id }
                    if self.target_concept_id.is_some_and(|target| target != concept_id)))
        {
            return Err(PublicContractError::InvalidFingerprint);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PublicBrowsePage {
    pub protocol_version: String,
    pub request_id: Uuid,
    pub manifest_sequence: u64,
    pub concepts: Vec<PublicConceptSummary>,
    pub next_cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<PublishedWikiWorkspacePage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<PublishedWikiDocument>,
}

impl PublicBrowsePage {
    pub fn validate_for(
        &self,
        request: &PublicBrowseRequest,
        publisher_id: &str,
    ) -> Result<(), PublicContractError> {
        let requested_after = request
            .cursor
            .as_deref()
            .map(Uuid::parse_str)
            .transpose()
            .map_err(|_| PublicContractError::InvalidLimit)?;
        let next_cursor = self
            .next_cursor
            .as_deref()
            .map(Uuid::parse_str)
            .transpose()
            .map_err(|_| PublicContractError::InvalidLimit)?;
        if self.protocol_version != request.protocol_version
            || self.request_id != request.request_id
            || self.concepts.len() > usize::from(request.limit)
            || (request.page.is_none()
                && request.target_concept_id.is_some_and(|target| {
                    self.concepts.first().map(|concept| concept.concept_id) != Some(target)
                }))
        {
            return Err(PublicContractError::InvalidLimit);
        }
        let mut previous_concept = requested_after;
        let legacy_protocol = request.protocol_version == PUBLIC_BROWSE_PROTOCOL;
        for concept in &self.concepts {
            if concept.publisher_id != publisher_id
                || concept.collection_id != request.collection_id
                || concept.tags.len() > 64
                || previous_concept.is_some_and(|previous| concept.concept_id <= previous)
                || if legacy_protocol {
                    concept.lifecycle_status.is_some()
                } else {
                    concept.lifecycle_status.as_deref() != Some("stable")
                }
            {
                return Err(PublicContractError::TooManyItems);
            }
            previous_concept = Some(concept.concept_id);
            validate_text(&concept.concept_type.to_string(), 120)?;
            validate_text(&concept.title, 240)?;
            validate_optional_text(&concept.description, 1_000)?;
            validate_text(&concept.language, 16)?;
            validate_optional_text(&concept.logical_resource_uri, 2_048)?;
            if concept.summary.chars().count() > MAX_SNIPPET_CHARS
                || concept.summary.chars().any(char::is_control)
            {
                return Err(PublicContractError::InvalidText);
            }
            for tag in &concept.tags {
                validate_text(tag, 64)?;
            }
        }
        let invalid_next_cursor = if legacy_protocol {
            next_cursor.is_some_and(|next| {
                requested_after.is_some_and(|previous| next <= previous)
                    || self
                        .concepts
                        .last()
                        .is_some_and(|concept| next < concept.concept_id)
            })
        } else {
            next_cursor.is_some()
                && (self.concepts.is_empty()
                    || next_cursor != self.concepts.last().map(|concept| concept.concept_id))
        };
        if invalid_next_cursor {
            return Err(PublicContractError::InvalidLimit);
        }
        crate::shared_wiki::validate_workspace_response(
            request.protocol_version == PUBLIC_BROWSE_PROTOCOL_V4,
            request.graph_cursor,
            request.page.as_ref(),
            &self.concepts,
            self.workspace.as_ref(),
            self.document.as_ref(),
        )
        .map_err(|_| PublicContractError::InvalidText)?;
        Ok(())
    }
}

impl PublishedConceptId for PublicConceptSummary {
    fn published_concept_id(&self) -> Uuid {
        self.concept_id
    }
}

fn valid_fingerprint(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn protocol_is_supported(value: &str, legacy: &str, current: &str) -> bool {
    value == legacy || value == current
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PublicContractError {
    #[error("unsupported public protocol")]
    UnsupportedProtocol,
    #[error("public request contains invalid text")]
    InvalidText,
    #[error("public request contains an invalid limit")]
    InvalidLimit,
    #[error("public manifest contains too many items")]
    TooManyItems,
    #[error("public manifest fingerprint is invalid")]
    InvalidFingerprint,
    #[error("public manifest is expired")]
    Expired,
    #[error("public manifest expiry is invalid")]
    InvalidExpiry,
}

fn validate_text(value: &str, max_bytes: usize) -> Result<(), PublicContractError> {
    if value.trim().is_empty() || value.len() > max_bytes || value.chars().any(char::is_control) {
        return Err(PublicContractError::InvalidText);
    }
    Ok(())
}

fn validate_optional_text(value: &str, max_bytes: usize) -> Result<(), PublicContractError> {
    if value.len() > max_bytes || value.chars().any(char::is_control) {
        return Err(PublicContractError::InvalidText);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    fn manifest() -> PublicCollectionManifest {
        let now = Utc::now();
        PublicCollectionManifest {
            protocol_version: PUBLIC_CATALOG_PROTOCOL.to_owned(),
            publisher_id: "publisher".to_owned(),
            collection_id: Uuid::new_v4(),
            sequence: 1,
            publication_fingerprint: "a".repeat(64),
            name: "Synthetic collection".to_owned(),
            description: "Bounded public profile".to_owned(),
            languages: vec!["en".to_owned()],
            concept_count: 1,
            okf_compatibility: None,
            routing_terms: vec!["synthetic".to_owned()],
            routes: vec!["/ip4/127.0.0.1/tcp/42042".to_owned()],
            updated_at: now,
            expires_at: now + Duration::minutes(15),
        }
    }

    #[test]
    fn public_manifest_rejects_oversized_profile_and_routing_payloads() {
        let now = Utc::now();
        let mut oversized_profile = manifest();
        oversized_profile.description = "x".repeat(1_001);
        assert!(oversized_profile.validate(now).is_err());

        let mut oversized_routing = manifest();
        oversized_routing.routing_terms = (0..=MAX_PUBLIC_ROUTING_TERMS)
            .map(|ordinal| format!("term-{ordinal}"))
            .collect();
        assert!(oversized_routing.validate(now).is_err());
    }

    #[test]
    fn public_manifest_rejects_unbounded_lifetime() {
        let now = Utc::now();
        let mut unbounded = manifest();
        unbounded.updated_at = now;
        unbounded.expires_at = now + Duration::seconds(MAX_PUBLIC_MANIFEST_LIFETIME_SECONDS + 1);

        assert_eq!(
            unbounded.validate(now),
            Err(PublicContractError::InvalidExpiry)
        );

        let mut future_dated = manifest();
        future_dated.updated_at =
            now + Duration::seconds(MAX_PUBLIC_MANIFEST_FUTURE_SKEW_SECONDS + 1);
        future_dated.expires_at = future_dated.updated_at + Duration::minutes(15);
        assert_eq!(
            future_dated.validate(now),
            Err(PublicContractError::InvalidExpiry)
        );
    }

    #[test]
    fn public_requests_reject_excessive_queries_collections_and_cursors() {
        let catalog = PublicCatalogQuery {
            protocol_version: PUBLIC_CATALOG_PROTOCOL.to_owned(),
            request_id: Uuid::new_v4(),
            query: "x".repeat(MAX_QUERY_BYTES + 1),
            languages: Vec::new(),
            limit: MAX_PUBLIC_CANDIDATES,
        };
        assert!(catalog.validate().is_err());

        let search = PublicSearchRequest {
            protocol_version: PUBLIC_SEARCH_PROTOCOL.to_owned(),
            request_id: Uuid::new_v4(),
            query: "synthetic".to_owned(),
            purpose: SearchPurpose::LocalAssistant,
            collections: (0..3)
                .map(|_| PublicCollectionTarget {
                    collection_id: Uuid::new_v4(),
                    manifest_sequence: 1,
                    publication_fingerprint: "a".repeat(64),
                })
                .collect(),
            top_k: MIN_TOP_K,
        };
        assert!(search.validate().is_err());

        let browse = PublicBrowseRequest {
            protocol_version: PUBLIC_BROWSE_PROTOCOL.to_owned(),
            request_id: Uuid::new_v4(),
            collection_id: Uuid::new_v4(),
            cursor: Some("x".repeat(129)),
            target_concept_id: None,
            graph_cursor: None,
            page: None,
            limit: MAX_PUBLIC_PAGE_SIZE,
        };
        assert!(browse.validate().is_err());
    }

    #[test]
    fn public_browse_page_rejects_excessive_or_cross_collection_results() {
        let publisher_id = "publisher";
        let collection_id = Uuid::new_v4();
        let request = PublicBrowseRequest {
            protocol_version: PUBLIC_BROWSE_PROTOCOL.to_owned(),
            request_id: Uuid::new_v4(),
            collection_id,
            cursor: None,
            target_concept_id: None,
            graph_cursor: None,
            page: None,
            limit: 1,
        };
        let concept = PublicConceptSummary {
            publisher_id: publisher_id.to_owned(),
            collection_id,
            concept_id: Uuid::new_v4(),
            concept_type: ConceptType::Document,
            title: "Synthetic concept".to_owned(),
            description: String::new(),
            language: "en".to_owned(),
            tags: Vec::new(),
            summary: "Bounded summary".to_owned(),
            logical_resource_uri: "urn:airwiki:synthetic".to_owned(),
            source_revision: 1,
            updated_at: Utc::now(),
            lifecycle_status: Some("stable".to_owned()),
            assurance: Some(crate::ConceptAssurance::default()),
        };
        let mut page = PublicBrowsePage {
            protocol_version: PUBLIC_BROWSE_PROTOCOL.to_owned(),
            request_id: request.request_id,
            manifest_sequence: 1,
            concepts: vec![concept.clone(), concept],
            next_cursor: None,
            workspace: None,
            document: None,
        };
        assert!(page.validate_for(&request, publisher_id).is_err());

        page.concepts.truncate(1);
        page.concepts[0].collection_id = Uuid::new_v4();
        assert!(page.validate_for(&request, publisher_id).is_err());
    }

    #[test]
    fn public_browse_anchor_must_be_the_first_returned_concept() {
        let publisher_id = "publisher";
        let collection_id = Uuid::new_v4();
        let target = Uuid::new_v4();
        let request = PublicBrowseRequest {
            protocol_version: PUBLIC_BROWSE_PROTOCOL_V3.to_owned(),
            request_id: Uuid::new_v4(),
            collection_id,
            cursor: None,
            target_concept_id: Some(target),
            graph_cursor: None,
            page: None,
            limit: 1,
        };
        let concept = PublicConceptSummary {
            publisher_id: publisher_id.to_owned(),
            collection_id,
            concept_id: target,
            concept_type: ConceptType::Document,
            title: "Anchored concept".to_owned(),
            description: String::new(),
            language: "en".to_owned(),
            tags: Vec::new(),
            summary: "Bounded summary".to_owned(),
            logical_resource_uri: "urn:airwiki:anchor".to_owned(),
            source_revision: 1,
            updated_at: Utc::now(),
            lifecycle_status: Some("stable".to_owned()),
            assurance: Some(crate::ConceptAssurance::default()),
        };
        let mut page = PublicBrowsePage {
            protocol_version: request.protocol_version.clone(),
            request_id: request.request_id,
            manifest_sequence: 1,
            concepts: vec![concept],
            next_cursor: None,
            workspace: None,
            document: None,
        };
        assert!(page.validate_for(&request, publisher_id).is_ok());

        page.concepts[0].concept_id = Uuid::new_v4();
        assert!(page.validate_for(&request, publisher_id).is_err());
    }

    #[test]
    fn public_browse_anchor_adapts_to_legacy_cursor_without_losing_exactness() {
        let target = Uuid::parse_str("10000000-0000-4000-8000-000000000001").unwrap();
        let mut request = PublicBrowseRequest {
            protocol_version: PUBLIC_BROWSE_PROTOCOL_V3.to_owned(),
            request_id: Uuid::new_v4(),
            collection_id: Uuid::new_v4(),
            cursor: None,
            target_concept_id: Some(target),
            graph_cursor: None,
            page: None,
            limit: 50,
        };

        request
            .prepare_for_protocol(PUBLIC_BROWSE_PROTOCOL_V2)
            .unwrap();

        assert_eq!(request.protocol_version, PUBLIC_BROWSE_PROTOCOL_V2);
        assert_eq!(request.target_concept_id, None);
        assert_eq!(
            request.cursor.as_deref(),
            Some("10000000-0000-4000-8000-000000000000")
        );
        assert!(request.validate().is_ok());
    }

    #[test]
    fn zero_public_browse_anchor_adapts_to_the_first_legacy_page() {
        let mut request = PublicBrowseRequest {
            protocol_version: PUBLIC_BROWSE_PROTOCOL_V3.to_owned(),
            request_id: Uuid::new_v4(),
            collection_id: Uuid::new_v4(),
            cursor: None,
            target_concept_id: Some(Uuid::nil()),
            graph_cursor: None,
            page: None,
            limit: 1,
        };

        request
            .prepare_for_protocol(PUBLIC_BROWSE_PROTOCOL)
            .unwrap();

        assert_eq!(request.cursor, None);
        assert_eq!(request.target_concept_id, None);
    }

    #[test]
    fn legacy_public_browse_contract_rejects_an_unadapted_anchor() {
        let request = PublicBrowseRequest {
            protocol_version: PUBLIC_BROWSE_PROTOCOL_V2.to_owned(),
            request_id: Uuid::new_v4(),
            collection_id: Uuid::new_v4(),
            cursor: None,
            target_concept_id: Some(Uuid::new_v4()),
            graph_cursor: None,
            page: None,
            limit: 1,
        };

        assert!(request.validate().is_err());
    }

    #[test]
    fn legacy_public_browse_accepts_a_cursor_past_filtered_concepts() {
        let publisher_id = "publisher";
        let collection_id = Uuid::new_v4();
        let concept_id = Uuid::from_u128(2);
        let request = PublicBrowseRequest {
            protocol_version: PUBLIC_BROWSE_PROTOCOL.to_owned(),
            request_id: Uuid::new_v4(),
            collection_id,
            cursor: Some(Uuid::from_u128(1).to_string()),
            target_concept_id: None,
            graph_cursor: None,
            page: None,
            limit: 2,
        };
        let page = PublicBrowsePage {
            protocol_version: request.protocol_version.clone(),
            request_id: request.request_id,
            manifest_sequence: 1,
            concepts: vec![PublicConceptSummary {
                publisher_id: publisher_id.to_owned(),
                collection_id,
                concept_id,
                concept_type: ConceptType::Document,
                title: "Legacy concept".to_owned(),
                description: String::new(),
                language: "en".to_owned(),
                tags: Vec::new(),
                summary: "Bounded summary".to_owned(),
                logical_resource_uri: "urn:airwiki:legacy".to_owned(),
                source_revision: 1,
                updated_at: Utc::now(),
                lifecycle_status: None,
                assurance: None,
            }],
            next_cursor: Some(Uuid::from_u128(3).to_string()),
            workspace: None,
            document: None,
        };

        assert!(page.validate_for(&request, publisher_id).is_ok());
    }
}
