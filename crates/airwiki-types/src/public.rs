use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    ConceptType, MAX_QUERY_BYTES, MAX_SNIPPET_CHARS, MAX_TOP_K, MIN_TOP_K, PUBLIC_BROWSE_PROTOCOL,
    PUBLIC_CATALOG_PROTOCOL, PUBLIC_SEARCH_PROTOCOL, SearchPurpose, SearchResponse,
};

pub const MAX_PUBLIC_PAGE_SIZE: u8 = 50;
pub const MAX_PUBLIC_CANDIDATES: u8 = 64;
pub const MAX_PUBLIC_ROUTING_TERMS: usize = 256;
pub const MAX_PUBLIC_ROUTES: usize = 8;

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
    pub routing_terms: Vec<String>,
    pub routes: Vec<String>,
    pub updated_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl PublicCollectionManifest {
    pub fn validate(&self, now: DateTime<Utc>) -> Result<(), PublicContractError> {
        if self.protocol_version != PUBLIC_CATALOG_PROTOCOL {
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
        if self.protocol_version != PUBLIC_CATALOG_PROTOCOL {
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
        if self.protocol_version != PUBLIC_SEARCH_PROTOCOL {
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
    pub limit: u8,
}

impl PublicBrowseRequest {
    pub fn validate(&self) -> Result<(), PublicContractError> {
        if self.protocol_version != PUBLIC_BROWSE_PROTOCOL {
            return Err(PublicContractError::UnsupportedProtocol);
        }
        if !(1..=MAX_PUBLIC_PAGE_SIZE).contains(&self.limit)
            || self
                .cursor
                .as_ref()
                .is_some_and(|cursor| cursor.len() > 128 || cursor.chars().any(char::is_control))
        {
            return Err(PublicContractError::InvalidLimit);
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
}

impl PublicBrowsePage {
    pub fn validate_for(
        &self,
        request: &PublicBrowseRequest,
        publisher_id: &str,
    ) -> Result<(), PublicContractError> {
        if self.protocol_version != PUBLIC_BROWSE_PROTOCOL
            || self.request_id != request.request_id
            || self.concepts.len() > usize::from(request.limit)
        {
            return Err(PublicContractError::InvalidLimit);
        }
        if self
            .next_cursor
            .as_ref()
            .is_some_and(|cursor| cursor.len() > 128 || cursor.chars().any(char::is_control))
        {
            return Err(PublicContractError::InvalidLimit);
        }
        for concept in &self.concepts {
            if concept.publisher_id != publisher_id
                || concept.collection_id != request.collection_id
                || concept.tags.len() > 64
            {
                return Err(PublicContractError::TooManyItems);
            }
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
        Ok(())
    }
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
        };
        let mut page = PublicBrowsePage {
            protocol_version: PUBLIC_BROWSE_PROTOCOL.to_owned(),
            request_id: request.request_id,
            manifest_sequence: 1,
            concepts: vec![concept.clone(), concept],
            next_cursor: None,
        };
        assert!(page.validate_for(&request, publisher_id).is_err());

        page.concepts.truncate(1);
        page.concepts[0].collection_id = Uuid::new_v4();
        assert!(page.validate_for(&request, publisher_id).is_err());
    }
}
