use std::net::SocketAddr;
use std::sync::Arc;

use airwiki_core::{
    Database, DeterministicEmbeddingProvider, DeterministicEvidenceRelevanceProvider,
    EmbeddingProvider, EvidenceDecision, EvidenceRelevanceError, EvidenceRelevanceProvider,
    HybridSearchEngine, OkfPublicationMaterializer, RelevanceInput, StoredChunk, sha256_file,
};
use airwiki_mcp::{MCP_PATH, McpServerConfig, start_mcp_server};
use airwiki_network::{MemorySecretStore, NodeIdentity};
use airwiki_types::{CollectionPolicy, ConceptType, EnrichmentDraft};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

struct RejectAllEvidence;

#[async_trait::async_trait]
impl EvidenceRelevanceProvider for RejectAllEvidence {
    fn profile_id(&self) -> &str {
        "reject-all-mcp-candidate-fixture"
    }

    async fn classify(
        &self,
        _question: &str,
        candidates: &[RelevanceInput],
    ) -> Result<Vec<EvidenceDecision>, EvidenceRelevanceError> {
        Ok(vec![EvidenceDecision::Irrelevant; candidates.len()])
    }
}

#[tokio::test]
async fn core_gate_distinguishes_supported_and_absent_facts_at_the_mcp_boundary() {
    let node_id = NodeIdentity::load_or_create(&MemorySecretStore::default())
        .unwrap()
        .peer_id()
        .to_string();
    let database = published_fixture(&node_id).await;
    let search = Arc::new(HybridSearchEngine::new(
        database,
        Arc::new(DeterministicEmbeddingProvider),
        Arc::new(DeterministicEvidenceRelevanceProvider),
        node_id,
    ));
    let server = start_mcp_server(McpServerConfig::default().with_port(0), search)
        .await
        .unwrap();
    let host = format!("127.0.0.1:{}", server.local_addr().port());
    let supported = raw_tool_call(
        server.local_addr(),
        &host,
        "¿Cómo se recupera el servicio de pagos?",
    )
    .await;
    assert!(supported.starts_with("HTTP/1.1 200"), "{supported}");
    assert!(supported.contains("\"status\":\"relevant_evidence\""));
    assert!(supported.contains("Reiniciar la cola de pagos"));

    let response =
        raw_tool_call(server.local_addr(), &host, "¿Cuál es el presupuesto anual?").await;

    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains("\"status\":\"no_relevant_evidence\""));
    assert!(response.contains("\"authorized_candidates\""));
    assert!(response.contains("Reiniciar la cola de pagos"));

    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn rejected_answer_remains_available_as_a_typed_external_ai_candidate() {
    let node_id = NodeIdentity::load_or_create(&MemorySecretStore::default())
        .unwrap()
        .peer_id()
        .to_string();
    let database = published_fixture(&node_id).await;
    let search = Arc::new(HybridSearchEngine::new(
        database,
        Arc::new(DeterministicEmbeddingProvider),
        Arc::new(RejectAllEvidence),
        node_id,
    ));
    let server = start_mcp_server(McpServerConfig::default().with_port(0), search)
        .await
        .unwrap();
    let host = format!("127.0.0.1:{}", server.local_addr().port());

    let response = raw_tool_call(
        server.local_addr(),
        &host,
        "¿Cómo se recupera el servicio de pagos?",
    )
    .await;

    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains("\"status\":\"no_relevant_evidence\""));
    assert!(response.contains("\"authorized_candidates\""));
    assert!(response.contains("Reiniciar la cola de pagos"));
    assert!(response.contains("\"heading_or_page\":\"Pasos\""));
    assert!(response.contains("\"source_revision\":1"));

    server.shutdown().await.unwrap();
}

async fn published_fixture(node_id: &str) -> Database {
    let database = Database::in_memory().unwrap();
    let root = tempfile::tempdir().unwrap();
    let source_folder = root.path().join("source");
    let wiki_folder = root.path().join("wiki");
    std::fs::create_dir_all(&source_folder).unwrap();
    std::fs::create_dir_all(&wiki_folder).unwrap();
    let source_path = source_folder.join("recovery.md");
    let text = "Reiniciar la cola de pagos y validar la API";
    std::fs::write(&source_path, text).unwrap();
    let source_sha256 = sha256_file(&source_path).unwrap();
    let collection = database
        .create_collection(
            "MCP relevance fixture",
            source_folder,
            wiki_folder,
            CollectionPolicy {
                local_only: false,
                peer_shareable: true,
                allow_external_ai: true,
            },
        )
        .unwrap();
    let source = database
        .register_source(
            collection.id,
            &source_path,
            &source_sha256,
            "markdown",
            u64::try_from(text.len()).unwrap(),
        )
        .unwrap();
    database
        .mark_extracted(source.id(), 0, u64::try_from(text.chars().count()).unwrap())
        .unwrap();
    let draft = EnrichmentDraft {
        concept_type: ConceptType::Runbook,
        title: "Recuperación de pagos".into(),
        description: "Procedimiento sintético de recuperación".into(),
        language: "es".into(),
        tags: vec!["pagos".into()],
        entities: Vec::new(),
        links: Vec::new(),
        summary: "Reiniciar la cola de pagos".into(),
        classification_confidence: 1.0,
        classification_explanation: "fixture".into(),
    };
    let concept = database
        .save_enrichment(source.id(), draft.clone(), node_id, "fixture")
        .unwrap();
    let embedding = DeterministicEmbeddingProvider
        .embed(&[format!("passage: {text}")])
        .await
        .unwrap()
        .remove(0);
    database
        .replace_chunks(
            concept.id,
            &[StoredChunk {
                id: Uuid::new_v4(),
                concept_id: concept.id,
                source_document_id: source.id(),
                collection_id: collection.id,
                ordinal: 0,
                heading_or_page: "Pasos".into(),
                text: text.into(),
                text_sha256: "b".repeat(64),
                embedding,
                source_revision: 1,
            }],
        )
        .unwrap();
    let review_version = database
        .review_evidence_page(concept.id, 1, None, None, 1)
        .unwrap()
        .unwrap()
        .review_version;
    OkfPublicationMaterializer::new(database.clone())
        .approve(concept.id, draft, &review_version)
        .unwrap();
    database
}

async fn raw_tool_call(address: SocketAddr, host: &str, question: &str) -> String {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "search_airwiki",
            "arguments": {
                "question": question,
                "top_k": 5
            }
        }
    })
    .to_string();
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let request = format!(
        "POST {MCP_PATH} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    String::from_utf8(response).unwrap()
}
