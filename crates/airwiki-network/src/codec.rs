//! Bounded CBOR codec for the public search protocol.

use std::io;

use airwiki_types::{
    MAX_RESPONSE_BYTES, SearchRequest, SearchResponse, SharedWikiBrowsePage,
    SharedWikiBrowseRequest,
};
use async_trait::async_trait;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use libp2p::StreamProtocol;
use libp2p::request_response::Codec;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

/// CBOR overhead plus the fixed request fields. The query itself is separately capped at 2 KiB.
pub const MAX_SEARCH_REQUEST_BYTES: usize = 4 * 1024;
pub const MAX_SHARED_WIKI_REQUEST_BYTES: usize = 2 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SearchWireResponse {
    Success(SearchResponse),
    Error(SearchWireError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SharedWikiWireResponse {
    Success(Box<SharedWikiBrowsePage>),
    Error(SearchWireError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchWireError {
    pub code: SearchWireErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchWireErrorCode {
    InvalidRequest,
    Unauthorized,
    RateLimited,
    Unavailable,
    Internal,
}

impl SearchWireError {
    pub fn new(code: SearchWireErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into().chars().take(500).collect(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct BoundedSearchCodec;

#[derive(Debug, Clone, Default)]
pub struct BoundedSharedWikiCodec;

fn encode_bounded<T: Serialize>(value: &T, limit: usize) -> io::Result<Vec<u8>> {
    let mut encoded = Vec::new();
    ciborium::ser::into_writer(value, &mut encoded)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    if encoded.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "CBOR payload is {} bytes; maximum is {limit}",
                encoded.len()
            ),
        ));
    }
    Ok(encoded)
}

async fn decode_bounded<T, R>(io: &mut R, limit: usize) -> io::Result<T>
where
    T: DeserializeOwned,
    R: AsyncRead + Unpin + Send,
{
    let mut encoded = Vec::new();
    io.take((limit + 1) as u64)
        .read_to_end(&mut encoded)
        .await?;
    if encoded.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("CBOR payload exceeds {limit} bytes"),
        ));
    }
    ciborium::de::from_reader(encoded.as_slice())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))
}

#[async_trait]
impl Codec for BoundedSearchCodec {
    type Protocol = StreamProtocol;
    type Request = SearchRequest;
    type Response = SearchWireResponse;

    async fn read_request<T>(&mut self, _: &Self::Protocol, io: &mut T) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        let request: SearchRequest = decode_bounded(io, MAX_SEARCH_REQUEST_BYTES).await?;
        request.validate().map_err(inbound_contract_error)?;
        Ok(request)
    }

    async fn read_response<T>(
        &mut self,
        _: &Self::Protocol,
        io: &mut T,
    ) -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send,
    {
        decode_bounded(io, MAX_RESPONSE_BYTES).await
    }

    async fn write_request<T>(
        &mut self,
        _: &Self::Protocol,
        io: &mut T,
        request: Self::Request,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        request
            .validate()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let encoded = encode_bounded(&request, MAX_SEARCH_REQUEST_BYTES)?;
        io.write_all(&encoded).await
    }

    async fn write_response<T>(
        &mut self,
        _: &Self::Protocol,
        io: &mut T,
        response: Self::Response,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        let encoded = encode_bounded(&response, MAX_RESPONSE_BYTES)?;
        io.write_all(&encoded).await
    }
}

#[async_trait]
impl Codec for BoundedSharedWikiCodec {
    type Protocol = StreamProtocol;
    type Request = SharedWikiBrowseRequest;
    type Response = SharedWikiWireResponse;

    async fn read_request<T>(
        &mut self,
        protocol: &Self::Protocol,
        io: &mut T,
    ) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        let request: SharedWikiBrowseRequest =
            decode_bounded(io, MAX_SHARED_WIKI_REQUEST_BYTES).await?;
        if request.protocol_version != protocol.as_ref() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "shared Wiki protocol does not match the negotiated protocol",
            ));
        }
        request.validate().map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "shared Wiki request is invalid")
        })?;
        Ok(request)
    }

    async fn read_response<T>(
        &mut self,
        protocol: &Self::Protocol,
        io: &mut T,
    ) -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send,
    {
        let response: SharedWikiWireResponse =
            decode_bounded(io, airwiki_types::MAX_SHARED_WIKI_RESPONSE_BYTES).await?;
        if let SharedWikiWireResponse::Success(page) = &response
            && page.protocol_version != protocol.as_ref()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "shared Wiki protocol does not match the negotiated protocol",
            ));
        }
        Ok(response)
    }

    async fn write_request<T>(
        &mut self,
        protocol: &Self::Protocol,
        io: &mut T,
        mut request: Self::Request,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        request
            .prepare_for_protocol(protocol.as_ref())
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        request
            .validate()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let encoded = encode_bounded(&request, MAX_SHARED_WIKI_REQUEST_BYTES)?;
        io.write_all(&encoded).await
    }

    async fn write_response<T>(
        &mut self,
        protocol: &Self::Protocol,
        io: &mut T,
        mut response: Self::Response,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        if let SharedWikiWireResponse::Success(page) = &mut response {
            page.prepare_for_protocol(protocol.as_ref())
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        }
        let encoded = encode_bounded(&response, airwiki_types::MAX_SHARED_WIKI_RESPONSE_BYTES)?;
        io.write_all(&encoded).await
    }
}

fn inbound_contract_error(error: airwiki_types::SearchContractError) -> io::Error {
    let message = match error {
        airwiki_types::SearchContractError::EmptyQuery => "search query is empty",
        airwiki_types::SearchContractError::QueryTooLarge(_) => "search query exceeds its limit",
        airwiki_types::SearchContractError::InvalidTopK(_) => "search result limit is invalid",
        airwiki_types::SearchContractError::UnsupportedProtocol(_) => {
            "search protocol is unsupported"
        }
        airwiki_types::SearchContractError::Unauthorized
        | airwiki_types::SearchContractError::Unavailable(_)
        | airwiki_types::SearchContractError::Backend(_) => "search request is invalid",
    };
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub fn response_fits(response: &SearchWireResponse) -> bool {
    encode_bounded(response, MAX_RESPONSE_BYTES).is_ok()
}

pub fn shared_wiki_response_fits(response: &SharedWikiWireResponse) -> bool {
    encode_bounded(response, airwiki_types::MAX_SHARED_WIKI_RESPONSE_BYTES).is_ok()
}

#[cfg(test)]
mod tests {
    use airwiki_types::{
        DEFAULT_TOP_K, MAX_SHARED_WIKI_DOCUMENT_BYTES, OkfCompatibility, PublishedWikiDocument,
        PublishedWikiPageDescriptor, PublishedWikiPageId, PublishedWikiPageRequest,
        PublishedWikiWorkspacePage, SHARED_WIKI_BROWSE_PROTOCOL_V2, SearchHit, SearchPurpose,
        SharedWikiDescriptor,
    };
    use chrono::Utc;
    use futures::io::Cursor;
    use libp2p::request_response::Codec as _;

    use super::*;

    #[derive(serde::Deserialize, serde::Serialize)]
    enum LegacySearchWireResponse {
        Success(LegacySearchResponse),
    }

    #[derive(serde::Deserialize, serde::Serialize)]
    struct LegacySearchResponse {
        request_id: uuid::Uuid,
        hits: Vec<SearchHit>,
        offline_nodes: Vec<String>,
        warnings: Vec<String>,
        partial: bool,
    }

    fn candidate() -> SearchHit {
        SearchHit {
            concept_id: uuid::Uuid::new_v4(),
            collection_id: uuid::Uuid::new_v4(),
            chunk_id: uuid::Uuid::new_v4(),
            title: "Candidate".to_owned(),
            snippet: "Authorized but unverified".to_owned(),
            heading_or_page: "Section".to_owned(),
            logical_resource_uri: "urn:airwiki:synthetic".to_owned(),
            source_revision: 1,
            source_sha256: "a".repeat(64),
            updated_at: Utc::now(),
            rank: 1,
            node_id: "synthetic".to_owned(),
            assurance: None,
            lifecycle_status: None,
        }
    }

    #[tokio::test]
    async fn codec_round_trips_valid_request() {
        let protocol = StreamProtocol::new(airwiki_types::SEARCH_PROTOCOL);
        let request = SearchRequest::new("pagos", SearchPurpose::LocalAssistant, DEFAULT_TOP_K);
        let mut bytes = Cursor::new(Vec::new());
        BoundedSearchCodec
            .write_request(&protocol, &mut bytes, request.clone())
            .await
            .unwrap();
        bytes.set_position(0);
        let decoded = BoundedSearchCodec
            .read_request(&protocol, &mut bytes)
            .await
            .unwrap();
        assert_eq!(decoded.request_id, request.request_id);
        assert_eq!(decoded.query, "pagos");
        assert_eq!(decoded.protocol_version, "/airwiki/search/2.0.0");
    }

    #[tokio::test]
    async fn codec_rejects_legacy_v1_request() {
        let protocol = StreamProtocol::new(airwiki_types::SEARCH_PROTOCOL);
        let mut request = SearchRequest::new("pagos", SearchPurpose::LocalAssistant, DEFAULT_TOP_K);
        request.protocol_version = "/airwiki/search/1.0.0".to_owned();
        let encoded = encode_bounded(&request, MAX_SEARCH_REQUEST_BYTES).unwrap();
        let mut bytes = Cursor::new(encoded);

        let error = BoundedSearchCodec
            .read_request(&protocol, &mut bytes)
            .await
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(error.to_string(), "search protocol is unsupported");
    }

    #[tokio::test]
    async fn codec_does_not_echo_untrusted_protocol_text_in_errors() {
        let protocol = StreamProtocol::new(airwiki_types::SEARCH_PROTOCOL);
        let canary = "CANARY\n/Users/alice/private/source.md";
        let mut request = SearchRequest::new("pagos", SearchPurpose::LocalAssistant, DEFAULT_TOP_K);
        request.protocol_version = canary.to_owned();
        let encoded = encode_bounded(&request, MAX_SEARCH_REQUEST_BYTES).unwrap();
        let mut bytes = Cursor::new(encoded);

        let error = BoundedSearchCodec
            .read_request(&protocol, &mut bytes)
            .await
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(error.to_string(), "search protocol is unsupported");
        assert!(!error.to_string().contains("CANARY"));
        assert!(!error.to_string().contains("/Users/alice"));
    }

    #[tokio::test]
    async fn codec_rejects_query_over_contract_limit() {
        let protocol = StreamProtocol::new(airwiki_types::SEARCH_PROTOCOL);
        let request = SearchRequest::new(
            "x".repeat(airwiki_types::MAX_QUERY_BYTES + 1),
            SearchPurpose::LocalAssistant,
            DEFAULT_TOP_K,
        );
        let mut bytes = Cursor::new(Vec::new());
        let error = BoundedSearchCodec
            .write_request(&protocol, &mut bytes, request)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[tokio::test]
    async fn codec_rejects_response_over_256_kib() {
        let protocol = StreamProtocol::new(airwiki_types::SEARCH_PROTOCOL);
        let mut response = SearchResponse::empty(uuid::Uuid::new_v4());
        response.warnings.push("x".repeat(MAX_RESPONSE_BYTES));
        let mut bytes = Cursor::new(Vec::new());
        let error = BoundedSearchCodec
            .write_response(&protocol, &mut bytes, SearchWireResponse::Success(response))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn codec_round_trips_authorized_candidates() {
        let protocol = StreamProtocol::new(airwiki_types::SEARCH_PROTOCOL);
        let mut response = SearchResponse::empty(uuid::Uuid::new_v4());
        response.authorized_candidates.push(candidate());
        let mut bytes = Cursor::new(Vec::new());
        BoundedSearchCodec
            .write_response(&protocol, &mut bytes, SearchWireResponse::Success(response))
            .await
            .unwrap();
        bytes.set_position(0);

        let decoded = BoundedSearchCodec
            .read_response(&protocol, &mut bytes)
            .await
            .unwrap();
        let SearchWireResponse::Success(decoded) = decoded else {
            panic!("expected success response");
        };
        assert_eq!(decoded.authorized_candidates.len(), 1);
    }

    #[tokio::test]
    async fn shared_wiki_codec_transfers_a_complete_maximum_size_page() {
        let collection_id = uuid::Uuid::new_v4();
        let mut request = SharedWikiBrowseRequest::new(collection_id, None, 1);
        let descriptor = PublishedWikiPageDescriptor {
            page: PublishedWikiPageId::Index,
            logical_path: "index.md".to_owned(),
            title: "Index".to_owned(),
            fingerprint: "a".repeat(64),
        };
        request.page = Some(PublishedWikiPageRequest {
            page: PublishedWikiPageId::Index,
            expected_fingerprint: Some(descriptor.fingerprint.clone()),
        });
        let page = SharedWikiBrowsePage {
            protocol_version: SHARED_WIKI_BROWSE_PROTOCOL_V2.to_owned(),
            request_id: request.request_id,
            wiki: SharedWikiDescriptor {
                collection_id,
                name: "Complete published Wiki".to_owned(),
                okf_compatibility: OkfCompatibility::DeclaredV02,
            },
            concepts: Vec::new(),
            next_cursor: None,
            workspace: Some(PublishedWikiWorkspacePage {
                reserved_pages: vec![descriptor.clone()],
                documents: Vec::new(),
                links: Vec::new(),
                next_graph_cursor: None,
            }),
            document: Some(PublishedWikiDocument {
                descriptor,
                body_markdown: "x".repeat(MAX_SHARED_WIKI_DOCUMENT_BYTES),
                metadata: Vec::new(),
                backlinks: Vec::new(),
                truncated: false,
            }),
        };
        assert!(page.validate_for(&request).is_ok());
        let response = SharedWikiWireResponse::Success(Box::new(page));
        assert!(shared_wiki_response_fits(&response));

        let protocol = StreamProtocol::new(SHARED_WIKI_BROWSE_PROTOCOL_V2);
        let mut bytes = Cursor::new(Vec::new());
        BoundedSharedWikiCodec
            .write_response(&protocol, &mut bytes, response)
            .await
            .unwrap();
        bytes.set_position(0);
        let decoded = BoundedSharedWikiCodec
            .read_response(&protocol, &mut bytes)
            .await
            .unwrap();
        let SharedWikiWireResponse::Success(decoded) = decoded else {
            panic!("expected complete shared Wiki response");
        };
        assert_eq!(
            decoded
                .document
                .as_ref()
                .map(|document| document.body_markdown.len()),
            Some(MAX_SHARED_WIKI_DOCUMENT_BYTES)
        );
    }

    #[tokio::test]
    async fn codec_defaults_candidates_from_legacy_response() {
        let protocol = StreamProtocol::new(airwiki_types::SEARCH_PROTOCOL);
        let request_id = uuid::Uuid::new_v4();
        let legacy = LegacySearchWireResponse::Success(LegacySearchResponse {
            request_id,
            hits: Vec::new(),
            offline_nodes: Vec::new(),
            warnings: Vec::new(),
            partial: false,
        });
        let mut bytes = Cursor::new(encode_bounded(&legacy, MAX_RESPONSE_BYTES).unwrap());

        let decoded = BoundedSearchCodec
            .read_response(&protocol, &mut bytes)
            .await
            .unwrap();
        let SearchWireResponse::Success(decoded) = decoded else {
            panic!("expected legacy success response");
        };
        assert_eq!(decoded.request_id, request_id);
        assert!(decoded.authorized_candidates.is_empty());
    }

    #[test]
    fn legacy_decoder_ignores_the_additive_candidate_field() {
        let mut response = SearchResponse::empty(uuid::Uuid::new_v4());
        response.authorized_candidates.push(candidate());
        let encoded =
            encode_bounded(&SearchWireResponse::Success(response), MAX_RESPONSE_BYTES).unwrap();

        let legacy: LegacySearchWireResponse = ciborium::from_reader(encoded.as_slice()).unwrap();

        assert!(matches!(legacy, LegacySearchWireResponse::Success(_)));
    }
}
