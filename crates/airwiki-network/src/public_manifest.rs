use airwiki_types::{
    PublicCollectionManifest, PublicCollectionTombstone, PublicContractError,
    SignedPublicCollectionManifest, SignedPublicCollectionTombstone,
};
use libp2p::identity::{Keypair, PublicKey};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PublicManifestError {
    #[error(transparent)]
    Contract(#[from] PublicContractError),
    #[error("public manifest encoding failed")]
    Encoding,
    #[error("public signing key is invalid")]
    InvalidPublicKey,
    #[error("public manifest identity does not match its signing key")]
    IdentityMismatch,
    #[error("public manifest signature is invalid")]
    InvalidSignature,
    #[error("public manifest signing failed")]
    Signing,
}

pub fn sign_manifest(
    keypair: &Keypair,
    manifest: PublicCollectionManifest,
) -> Result<SignedPublicCollectionManifest, PublicManifestError> {
    manifest.validate(chrono::Utc::now())?;
    ensure_publisher(keypair, &manifest.publisher_id)?;
    let bytes = canonical_bytes(&manifest)?;
    let signature = keypair
        .sign(&bytes)
        .map_err(|_| PublicManifestError::Signing)?;
    Ok(SignedPublicCollectionManifest {
        manifest,
        public_key: keypair.public().encode_protobuf(),
        signature,
    })
}

pub fn verify_manifest(
    signed: &SignedPublicCollectionManifest,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), PublicManifestError> {
    signed.manifest.validate(now)?;
    let public = decode_public(&signed.public_key)?;
    if public.to_peer_id().to_string() != signed.manifest.publisher_id {
        return Err(PublicManifestError::IdentityMismatch);
    }
    let bytes = canonical_bytes(&signed.manifest)?;
    if !public.verify(&bytes, &signed.signature) {
        return Err(PublicManifestError::InvalidSignature);
    }
    Ok(())
}

pub fn sign_tombstone(
    keypair: &Keypair,
    tombstone: PublicCollectionTombstone,
) -> Result<SignedPublicCollectionTombstone, PublicManifestError> {
    ensure_publisher(keypair, &tombstone.publisher_id)?;
    let bytes = canonical_bytes(&tombstone)?;
    let signature = keypair
        .sign(&bytes)
        .map_err(|_| PublicManifestError::Signing)?;
    Ok(SignedPublicCollectionTombstone {
        tombstone,
        public_key: keypair.public().encode_protobuf(),
        signature,
    })
}

pub fn verify_tombstone(
    signed: &SignedPublicCollectionTombstone,
) -> Result<(), PublicManifestError> {
    if signed.tombstone.protocol_version != airwiki_types::PUBLIC_CATALOG_PROTOCOL {
        return Err(PublicContractError::UnsupportedProtocol.into());
    }
    let public = decode_public(&signed.public_key)?;
    if public.to_peer_id().to_string() != signed.tombstone.publisher_id {
        return Err(PublicManifestError::IdentityMismatch);
    }
    let bytes = canonical_bytes(&signed.tombstone)?;
    if !public.verify(&bytes, &signed.signature) {
        return Err(PublicManifestError::InvalidSignature);
    }
    Ok(())
}

fn ensure_publisher(keypair: &Keypair, publisher_id: &str) -> Result<(), PublicManifestError> {
    if keypair.public().to_peer_id().to_string() != publisher_id {
        return Err(PublicManifestError::IdentityMismatch);
    }
    Ok(())
}

fn decode_public(encoded: &[u8]) -> Result<PublicKey, PublicManifestError> {
    PublicKey::try_decode_protobuf(encoded).map_err(|_| PublicManifestError::InvalidPublicKey)
}

fn canonical_bytes(value: &impl serde::Serialize) -> Result<Vec<u8>, PublicManifestError> {
    let mut bytes = Vec::new();
    ciborium::into_writer(value, &mut bytes).map_err(|_| PublicManifestError::Encoding)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};
    use uuid::Uuid;

    use super::*;

    fn manifest(keypair: &Keypair) -> PublicCollectionManifest {
        PublicCollectionManifest {
            protocol_version: airwiki_types::PUBLIC_CATALOG_PROTOCOL.to_owned(),
            publisher_id: keypair.public().to_peer_id().to_string(),
            collection_id: Uuid::new_v4(),
            sequence: 1,
            publication_fingerprint: "a".repeat(64),
            name: "Atlas".to_owned(),
            description: "Synthetic public knowledge".to_owned(),
            languages: vec!["es".to_owned()],
            concept_count: 1,
            routing_terms: vec!["atlas".to_owned()],
            routes: vec!["/ip4/127.0.0.1/udp/41000/quic-v1".to_owned()],
            updated_at: Utc::now(),
            expires_at: Utc::now() + Duration::minutes(15),
        }
    }

    #[test]
    fn signed_manifest_rejects_tampering() {
        let keypair = Keypair::generate_ed25519();
        let mut signed = sign_manifest(&keypair, manifest(&keypair)).unwrap();
        verify_manifest(&signed, Utc::now()).unwrap();
        signed.manifest.name = "Tampered".to_owned();
        assert!(matches!(
            verify_manifest(&signed, Utc::now()),
            Err(PublicManifestError::InvalidSignature)
        ));
    }

    #[test]
    fn manifest_rejects_another_publishers_identity() {
        let keypair = Keypair::generate_ed25519();
        let other = Keypair::generate_ed25519();
        assert!(matches!(
            sign_manifest(&other, manifest(&keypair)),
            Err(PublicManifestError::IdentityMismatch)
        ));
    }
}
