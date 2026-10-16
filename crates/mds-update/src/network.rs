use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use mds_wire::{NetworkMap, WireEnvelope};

use crate::{decode_key, decode_signature, hex_encode, sha256_hex, UpdateError, PINNED_PUBLIC_HEX};

pub fn node_id_of(public_hex: &str) -> Result<String, UpdateError> {
    let public = decode_key(public_hex).ok_or(UpdateError::BadKey)?;
    Ok(sha256_hex(&public))
}

pub fn sign_network(secret_hex: &str, map: &mut NetworkMap) -> Result<(), UpdateError> {
    map.signature.clear();
    let body = serde_json::to_vec(map)?;
    let secret = decode_key(secret_hex).ok_or(UpdateError::BadKey)?;
    let signing = SigningKey::from_bytes(&secret);
    map.signature = hex_encode(&signing.sign(&body).to_bytes());
    Ok(())
}

pub fn verify_network(map: &NetworkMap) -> Result<(), UpdateError> {
    verify_network_with(map, PINNED_PUBLIC_HEX)
}

pub fn verify_network_with(map: &NetworkMap, public_hex: &str) -> Result<(), UpdateError> {
    let signature = decode_signature(&map.signature).ok_or(UpdateError::BadSignature)?;
    let mut unsigned = map.clone();
    unsigned.signature.clear();
    let body = serde_json::to_vec(&unsigned)?;
    let public = decode_key(public_hex).ok_or(UpdateError::BadKey)?;
    let verifying = VerifyingKey::from_bytes(&public).map_err(|_| UpdateError::BadKey)?;
    verifying
        .verify(&body, &signature)
        .map_err(|_| UpdateError::BadSignature)
}

pub fn seal(secret_hex: &str, envelope: &mut WireEnvelope, body: &[u8]) -> Result<(), UpdateError> {
    envelope.body_sha256 = sha256_hex(body);
    envelope.signature.clear();
    let message = wire_message(envelope);
    let secret = decode_key(secret_hex).ok_or(UpdateError::BadKey)?;
    let signing = SigningKey::from_bytes(&secret);
    envelope.signature = hex_encode(&signing.sign(&message).to_bytes());
    Ok(())
}

pub fn open_wire(envelope: &WireEnvelope, body: &[u8], public_hex: &str) -> Result<(), UpdateError> {
    if sha256_hex(body) != envelope.body_sha256 {
        return Err(UpdateError::HashMismatch);
    }
    let signature = decode_signature(&envelope.signature).ok_or(UpdateError::BadSignature)?;
    let mut unsigned = envelope.clone();
    unsigned.signature.clear();
    let public = decode_key(public_hex).ok_or(UpdateError::BadKey)?;
    let verifying = VerifyingKey::from_bytes(&public).map_err(|_| UpdateError::BadKey)?;
    verifying
        .verify(&wire_message(&unsigned), &signature)
        .map_err(|_| UpdateError::BadSignature)
}

fn wire_message(envelope: &WireEnvelope) -> Vec<u8> {
    format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
        envelope.protocol_version,
        envelope.node_id,
        envelope.request_id,
        envelope.sent_at,
        envelope.nonce,
        envelope.model_generation,
        envelope.index_generation,
        envelope.kind,
        envelope.body_sha256
    )
    .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate_keypair;
    use mds_wire::{HiveDoor, PlaceEntry, PlaceRef};

    fn sample(public: &str) -> NetworkMap {
        NetworkMap {
            version: 1,
            places: vec![PlaceEntry {
                place: PlaceRef {
                    country: "Беларусь".into(),
                    region: "Минск".into(),
                    city: "Минск".into(),
                },
                hives: vec![HiveDoor {
                    base: "https://minsk.example".into(),
                }],
                shards: Vec::new(),
            }],
            keys: vec![mds_wire::NodeKey {
                node_id: node_id_of(public).unwrap(),
                public_hex: public.into(),
                role: "full".into(),
            }],
            signature: String::new(),
        }
    }

    #[test]
    fn unsigned_map_is_rejected_and_signed_map_opens() {
        let (secret, public) = generate_keypair();
        let mut map = sample(&public);
        assert!(verify_network_with(&map, &public).is_err());
        sign_network(&secret, &mut map).unwrap();
        verify_network_with(&map, &public).unwrap();
        map.places[0].place.city = "Брест".into();
        assert!(verify_network_with(&map, &public).is_err());
    }

    #[test]
    fn wire_seal_rejects_a_changed_body() {
        let (secret, public) = generate_keypair();
        let mut envelope = WireEnvelope {
            protocol_version: 1,
            node_id: node_id_of(&public).unwrap(),
            request_id: "a".into(),
            sent_at: 1,
            nonce: "n".into(),
            model_generation: "e5".into(),
            index_generation: 1,
            kind: "search".into(),
            body_sha256: String::new(),
            signature: String::new(),
        };
        seal(&secret, &mut envelope, b"{\"text\":\"honey\"}").unwrap();
        open_wire(&envelope, b"{\"text\":\"honey\"}", &public).unwrap();
        assert!(open_wire(&envelope, b"{\"text\":\"other\"}", &public).is_err());
    }
}
