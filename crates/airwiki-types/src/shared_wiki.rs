//! Bounded summaries used to browse an explicitly granted Wiki over the LAN.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    ConceptAssurance, ConceptType, MAX_SNIPPET_CHARS, OkfCompatibility, SHARED_WIKI_BROWSE_PROTOCOL,
};

/// Maximum concepts returned by one authenticated LAN browse request.
pub const MAX_SHARED_WIKI_PAGE_SIZE: u8 = 50;

/// Requests one bounded page from a Wiki already granted to the authenticated peer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedWikiBrowseRequest {
    pub protocol_version: String,
    pub request_id: Uuid,
    pub collection_id: Uuid,
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_concept_id: Option<Uuid>,
    pub limit: u8,
}

impl SharedWikiBrowseRequest {
    pub fn new(collection_id: Uuid, cursor: Option<String>, limit: u8) -> Self {
        Self {
            protocol_version: SHARED_WIKI_BROWSE_PROTOCOL.to_owned(),
            request_id: Uuid::new_v4(),
            collection_id,
            cursor,
            target_concept_id: None,
            limit,
        }
    }

    pub fn from_concept(collection_id: Uuid, target_concept_id: Uuid, limit: u8) -> Self {
        let mut request = Self::new(collection_id, None, limit);
        request.target_concept_id = Some(target_concept_id);
        request
    }

    pub fn validate(&self) -> Result<(), SharedWikiContractError> {
        if self.protocol_version != SHARED_WIKI_BROWSE_PROTOCOL {
            return Err(SharedWikiContractError::UnsupportedProtocol);
        }
        if !(1..=MAX_SHARED_WIKI_PAGE_SIZE).contains(&self.limit)
            || (self.cursor.is_some() && self.target_concept_id.is_some())
            || self
                .cursor
                .as_deref()
                .is_some_and(|cursor| Uuid::parse_str(cursor).is_err())
        {
            return Err(SharedWikiContractError::InvalidPage);
        }
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

/// Read-only concept metadata. Complete documents and source paths never cross this contract.
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
            || request.target_concept_id.is_some_and(|target| {
                self.concepts.first().map(|concept| concept.concept_id) != Some(target)
            })
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
        Ok(())
    }
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
        SharedWikiBrowsePage {
            protocol_version: request.protocol_version.clone(),
            request_id: request.request_id,
            wiki: SharedWikiDescriptor {
                collection_id: request.collection_id,
                name: "Atlas compartido".to_owned(),
                okf_compatibility: OkfCompatibility::DeclaredV02,
            },
            concepts: vec![SharedWikiConceptSummary {
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
            }],
            next_cursor: None,
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
}
