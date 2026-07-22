use std::process::ExitCode;
use std::time::{Duration, Instant};

use airwiki_federation_index::CatalogStore;
use airwiki_network::{Keypair, sign_manifest};
use airwiki_types::{
    MAX_PUBLIC_CANDIDATES, PUBLIC_CATALOG_PROTOCOL, PublicCatalogQuery, PublicCollectionManifest,
};
use chrono::{Duration as ChronoDuration, Utc};
use uuid::Uuid;

const PUBLISHERS: u32 = 10_000;
const COLLECTIONS_PER_PUBLISHER: u32 = 10;
const QUERY_SAMPLES: u32 = 100;
const P95_GATE: Duration = Duration::from_millis(1_500);

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let store = CatalogStore::in_memory()?;
    let now = Utc::now();
    let load_started = Instant::now();
    for publisher in 0..PUBLISHERS {
        let keypair = Keypair::generate_ed25519();
        let publisher_id = keypair.public().to_peer_id().to_string();
        for offset in 0..COLLECTIONS_PER_PUBLISHER {
            let ordinal = u128::from(publisher) * u128::from(COLLECTIONS_PER_PUBLISHER)
                + u128::from(offset)
                + 1;
            let topic = format!("topic{:04}", ordinal % 1_000);
            let manifest = sign_manifest(
                &keypair,
                PublicCollectionManifest {
                    protocol_version: PUBLIC_CATALOG_PROTOCOL.to_owned(),
                    publisher_id: publisher_id.clone(),
                    collection_id: Uuid::from_u128(ordinal),
                    sequence: 1,
                    publication_fingerprint: format!("{ordinal:064x}"),
                    name: format!("Synthetic collection {ordinal}"),
                    description: "Reproducible public routing benchmark".to_owned(),
                    languages: vec!["en".to_owned()],
                    concept_count: 10,
                    routing_terms: vec![topic],
                    routes: vec!["/ip4/127.0.0.1/tcp/42043".to_owned()],
                    updated_at: now,
                    expires_at: now + ChronoDuration::hours(2),
                },
            )?;
            store.register(&manifest, now)?;
        }
    }
    let mut samples = Vec::with_capacity(QUERY_SAMPLES as usize);
    for sample in 0..QUERY_SAMPLES {
        let query = PublicCatalogQuery {
            protocol_version: PUBLIC_CATALOG_PROTOCOL.to_owned(),
            request_id: Uuid::from_u128(u128::from(sample) + 1),
            query: format!("topic{:04}", sample % 1_000),
            languages: vec!["en".to_owned()],
            limit: MAX_PUBLIC_CANDIDATES,
        };
        let started = Instant::now();
        let results = store.query(&query, Utc::now())?;
        let elapsed = started.elapsed();
        if results.len() > usize::from(MAX_PUBLIC_CANDIDATES) {
            return Err("catalog returned more than the candidate limit".into());
        }
        samples.push(elapsed);
    }
    samples.sort_unstable();
    let p95_index = (samples.len() * 95 / 100).min(samples.len().saturating_sub(1));
    let p95 = samples[p95_index];
    println!(
        "publishers={PUBLISHERS} collections={} load_ms={} query_p95_ms={} gate_ms={}",
        PUBLISHERS * COLLECTIONS_PER_PUBLISHER,
        load_started.elapsed().as_millis(),
        p95.as_millis(),
        P95_GATE.as_millis()
    );
    if p95 >= P95_GATE {
        return Err(format!("catalog p95 {:?} exceeded gate {:?}", p95, P95_GATE).into());
    }
    Ok(())
}
