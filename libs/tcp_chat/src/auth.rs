use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::error::{LanChatError, Result};
use crate::protocol::PROTOCOL_VERSION;

type HmacSha256 = Hmac<Sha256>;

pub const DEFAULT_ROOM_NAME: &str = "default";
pub const AUTH_MODE_NONE: &str = "none";
pub const AUTH_MODE_HMAC_SHA256: &str = "hmac-sha256";

pub fn normalize_room_name(room_name: String) -> Result<String> {
    let room_name = room_name.trim();
    if room_name.is_empty() {
        return Err(LanChatError::Config(
            "room name must contain at least one visible character".to_string(),
        ));
    }

    Ok(room_name.to_string())
}

pub fn normalize_shared_secret(shared_secret: Option<String>) -> Result<Option<String>> {
    let Some(shared_secret) = shared_secret else {
        return Ok(None);
    };

    let shared_secret = shared_secret.trim();
    if shared_secret.is_empty() {
        return Err(LanChatError::Config(
            "shared secret must not be empty when provided".to_string(),
        ));
    }

    Ok(Some(shared_secret.to_string()))
}

pub fn derive_room_id(room_name: &str) -> String {
    let canonical_room = room_name
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();

    hex_encode(&Sha256::digest(canonical_room.as_bytes()))
}

pub fn auth_mode(shared_secret: Option<&str>) -> &'static str {
    if shared_secret.is_some() {
        AUTH_MODE_HMAC_SHA256
    } else {
        AUTH_MODE_NONE
    }
}

pub fn build_auth_proof(
    shared_secret: Option<&str>,
    room_id: &str,
    peer_id: &str,
    display_name: &str,
) -> Result<Option<String>> {
    let Some(shared_secret) = shared_secret else {
        return Ok(None);
    };

    let mut mac = HmacSha256::new_from_slice(shared_secret.as_bytes()).map_err(|error| {
        LanChatError::Protocol(format!("failed to initialize auth proof: {error}"))
    })?;
    mac.update(&[PROTOCOL_VERSION]);
    mac.update(b"|");
    mac.update(room_id.as_bytes());
    mac.update(b"|");
    mac.update(peer_id.as_bytes());
    mac.update(b"|");
    mac.update(display_name.as_bytes());

    Ok(Some(hex_encode(&mac.finalize().into_bytes())))
}

pub fn verify_auth_proof(
    shared_secret: Option<&str>,
    room_id: &str,
    peer_id: &str,
    display_name: &str,
    provided_proof: Option<&str>,
) -> Result<()> {
    match (shared_secret, provided_proof) {
        (None, None) => Ok(()),
        (None, Some(_)) => Err(LanChatError::Protocol(
            "peer requires shared-secret authentication but this node does not".to_string(),
        )),
        (Some(_), None) => Err(LanChatError::Protocol(
            "peer did not provide the required shared-secret proof".to_string(),
        )),
        (Some(shared_secret), Some(provided_proof)) => {
            let expected = build_auth_proof(Some(shared_secret), room_id, peer_id, display_name)?
                .expect("auth proof should be generated when a secret is provided");

            if bool::from(expected.as_bytes().ct_eq(provided_proof.as_bytes())) {
                Ok(())
            } else {
                Err(LanChatError::Protocol(
                    "shared-secret authentication failed".to_string(),
                ))
            }
        }
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }

    encoded
}

#[cfg(test)]
mod tests {
    use super::{
        auth_mode, build_auth_proof, derive_room_id, normalize_room_name, normalize_shared_secret,
        verify_auth_proof, AUTH_MODE_HMAC_SHA256, AUTH_MODE_NONE,
    };

    #[test]
    fn room_ids_are_case_insensitive() {
        assert_eq!(derive_room_id("Studio A"), derive_room_id(" studio   a "));
    }

    #[test]
    fn shared_secrets_are_trimmed_and_validated() {
        let secret = normalize_shared_secret(Some("  secret  ".to_string()))
            .expect("secret should be valid");
        assert_eq!(secret.as_deref(), Some("secret"));
        assert!(normalize_shared_secret(Some("   ".to_string())).is_err());
    }

    #[test]
    fn auth_proofs_round_trip() {
        let proof = build_auth_proof(Some("room-key"), "room-id", "peer-1", "Cuervo")
            .expect("proof should build");

        verify_auth_proof(
            Some("room-key"),
            "room-id",
            "peer-1",
            "Cuervo",
            proof.as_deref(),
        )
        .expect("proof should verify");
    }

    #[test]
    fn wrong_secret_is_rejected() {
        let proof = build_auth_proof(Some("room-key"), "room-id", "peer-1", "Cuervo")
            .expect("proof should build");

        assert!(verify_auth_proof(
            Some("wrong-key"),
            "room-id",
            "peer-1",
            "Cuervo",
            proof.as_deref(),
        )
        .is_err());
    }

    #[test]
    fn auth_modes_reflect_config() {
        assert_eq!(auth_mode(None), AUTH_MODE_NONE);
        assert_eq!(auth_mode(Some("secret")), AUTH_MODE_HMAC_SHA256);
    }

    #[test]
    fn room_names_must_not_be_empty() {
        assert!(normalize_room_name("  ".to_string()).is_err());
    }
}
