//! Sunshine-specific domains and lookup digests over xcss secret types.

use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use hkdf::Hkdf;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use xcss::secret::{SecretBytes, SecretKey};
use xcss::secret_envelope::EnvelopeDomain;

use crate::error::{AppError, AppResult};

use crate::persistent_crypto_definition::*;

#[derive(Clone)]
pub struct SecretBox {
    current_id: String,
    current: Arc<SecretKey<32>>,
    request_fingerprint_key: [u8; 32],
    idempotency_key_hash_key: [u8; 32],
}

impl SecretBox {
    pub fn new(current_id: impl Into<String>, current: [u8; 32]) -> anyhow::Result<Self> {
        let current_id = validate_key_id(current_id.into())?;
        let derivation = Hkdf::<Sha256>::new(Some(HKDF_SALT), &current);
        let request_fingerprint_key =
            derive_key(&derivation, REQUEST_FINGERPRINT_INFO, "request fingerprint")?;
        let idempotency_key_hash_key = derive_key(
            &derivation,
            IDEMPOTENCY_KEY_HASH_INFO,
            "idempotency key hash",
        )?;
        Ok(Self {
            current_id,
            current: Arc::new(SecretKey::new(current)),
            request_fingerprint_key,
            idempotency_key_hash_key,
        })
    }

    /// Seal one durable operation request. Both the row identity and declared
    /// action are authenticated so neither cross-row nor cross-action swaps are
    /// accepted before the strict request enum is parsed.
    pub fn encrypt_operation_request(
        &self,
        operation_id: &str,
        action: &str,
        value: &str,
    ) -> AppResult<String> {
        self.encrypt::<OperationRequestEnvelope>(
            value,
            &operation_request_aad(operation_id, action),
        )
    }

    pub fn decrypt_operation_request(
        &self,
        operation_id: &str,
        action: &str,
        value: &str,
    ) -> AppResult<String> {
        self.decrypt::<OperationRequestEnvelope>(
            value,
            &operation_request_aad(operation_id, action),
        )
    }

    pub fn encrypt_client_authorization(&self, device_id: &str, value: &str) -> AppResult<String> {
        self.encrypt::<ClientAuthorizationEnvelope>(value, device_id.as_bytes())
    }

    pub fn decrypt_client_authorization(&self, device_id: &str, value: &str) -> AppResult<String> {
        self.decrypt::<ClientAuthorizationEnvelope>(value, device_id.as_bytes())
    }

    fn encrypt<D: EnvelopeDomain>(&self, value: &str, binding: &[u8]) -> AppResult<String> {
        let payload = xcss::secret_envelope::seal::<D>(
            &self.current,
            binding,
            &SecretBytes::new(value.as_bytes().to_vec()),
        )
        .map_err(|_| AppError::Crypto)?;
        Ok(format!(
            "{PREFIX}{}:{}",
            self.current_id,
            STANDARD.encode(payload)
        ))
    }

    fn decrypt<D: EnvelopeDomain>(&self, value: &str, binding: &[u8]) -> AppResult<String> {
        let rest = value.strip_prefix(PREFIX).ok_or(AppError::Crypto)?;
        let (id, payload) = rest.split_once(':').ok_or(AppError::Crypto)?;
        if id != self.current_id {
            return Err(AppError::Crypto);
        }
        let payload = decode_payload(payload)?;
        let plaintext = xcss::secret_envelope::open::<D>(&self.current, binding, &payload)
            .map_err(|_| AppError::Crypto)?;
        String::from_utf8(plaintext.expose().to_vec()).map_err(|_| AppError::Crypto)
    }

    /// Stable keyed fingerprint used only to decide whether a repeated
    /// Idempotency-Key carries the exact same canonical request JSON.
    pub fn operation_request_fingerprint(&self, canonical_request: &str) -> [u8; 32] {
        hmac_sha256(&self.request_fingerprint_key, canonical_request.as_bytes())
    }

    /// Stable, non-reversible database lookup key for a caller-provided
    /// Idempotency-Key. Its HKDF info is deliberately distinct from request
    /// fingerprints, so equal input bytes cannot cross protocol domains.
    pub fn operation_idempotency_key_hash(&self, idempotency_key: &str) -> [u8; 32] {
        hmac_sha256(&self.idempotency_key_hash_key, idempotency_key.as_bytes())
    }
}

pub(crate) fn constant_time_equal_32(left: &[u8], right: &[u8; 32]) -> bool {
    let Ok(left) = <&[u8; 32]>::try_from(left) else {
        return false;
    };
    bool::from(left.as_slice().ct_eq(right.as_slice()))
}

fn derive_key(derivation: &Hkdf<Sha256>, info: &[u8], purpose: &str) -> anyhow::Result<[u8; 32]> {
    let mut derived = [0_u8; 32];
    derivation
        .expand(info, &mut derived)
        .map_err(|_| anyhow::anyhow!("HKDF output length is invalid for {purpose}"))?;
    Ok(derived)
}

fn hmac_sha256(key: &[u8; 32], value: &[u8]) -> [u8; 32] {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key)
        .expect("an HMAC-SHA-256 key accepts the derived 32-byte key");
    mac.update(value);
    mac.finalize().into_bytes().into()
}

/// Length framing makes the deterministic context injective even if a future
/// identifier or action is allowed to contain a separator used by another
/// component. The bytes are deliberately independent of serde or display
/// formatting because they are part of the current persistent crypto contract.
fn authenticated_context(domain: &[u8], components: &[&[u8]]) -> Vec<u8> {
    let mut context = Vec::with_capacity(
        AAD_FORMAT.len()
            + domain.len()
            + components
                .iter()
                .map(|value| value.len() + 8)
                .sum::<usize>()
            + 16,
    );
    append_context_component(&mut context, AAD_FORMAT);
    append_context_component(&mut context, domain);
    for component in components {
        append_context_component(&mut context, component);
    }
    context
}

fn append_context_component(context: &mut Vec<u8>, value: &[u8]) {
    let length = u64::try_from(value.len()).expect("AAD component length must fit u64");
    context.extend_from_slice(&length.to_be_bytes());
    context.extend_from_slice(value);
}

fn operation_request_aad(operation_id: &str, action: &str) -> Vec<u8> {
    authenticated_context(
        OPERATION_REQUEST_DOMAIN,
        &[
            operation_id.as_bytes(),
            action.as_bytes(),
            OPERATION_REQUEST_FIELD,
        ],
    )
}

fn decode_payload(value: &str) -> AppResult<Vec<u8>> {
    STANDARD.decode(value).map_err(|_| AppError::Crypto)
}

fn validate_key_id(value: String) -> anyhow::Result<String> {
    anyhow::ensure!(
        !value.is_empty()
            && value.len() <= 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
        "key id must contain 1-64 ASCII letters, digits, '-' or '_'"
    );
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn operation_envelope_binds_id_action_and_key() {
        let secret = SecretBox::new("one", [3; 32]).unwrap();
        let a = secret
            .encrypt_operation_request("op-a", "sunshine.config.patch", "payload")
            .unwrap();
        let b = secret
            .encrypt_operation_request("op-a", "sunshine.config.patch", "payload")
            .unwrap();
        assert_ne!(a, b);
        assert!(a.starts_with("xscs:sgev1:"));
        // Do not accept a foreign outer prefix as
        // an alias, even if the authenticated payload uses the current domain.
        let foreign_namespace = a.replacen("xscs:sgev1:", "foreign:sgev1:", 1);
        assert!(
            secret
                .decrypt_operation_request("op-a", "sunshine.config.patch", &foreign_namespace)
                .is_err()
        );
        assert_eq!(
            secret
                .decrypt_operation_request("op-a", "sunshine.config.patch", &a)
                .unwrap(),
            "payload"
        );
        assert!(
            secret
                .decrypt_operation_request("op-b", "sunshine.config.patch", &a)
                .is_err()
        );
        assert!(
            secret
                .decrypt_operation_request("op-a", "sunshine.restart", &a)
                .is_err()
        );
        assert!(
            SecretBox::new("two", [4; 32])
                .unwrap()
                .decrypt_operation_request("op-a", "sunshine.config.patch", &a)
                .is_err()
        );
    }
    #[test]
    fn operation_digests_are_keyed_and_domain_separated() {
        let a = SecretBox::new("one", [3; 32]).unwrap();
        let b = SecretBox::new("two", [4; 32]).unwrap();
        assert_ne!(
            a.operation_request_fingerprint("x"),
            a.operation_idempotency_key_hash("x")
        );
        assert_ne!(
            a.operation_request_fingerprint("x"),
            b.operation_request_fingerprint("x")
        );
        assert_eq!(
            a.operation_request_fingerprint("x"),
            a.operation_request_fingerprint("x")
        );
    }
}

#[cfg(test)]
mod installation_vector_tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;
    #[test]
    fn current_authorization_material_matches_runtime_decryption() {
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../../schema/validation/authorization.json"
        ))
        .unwrap();
        for case in cases.as_array().unwrap() {
            let input = &case["input"];
            let valid = (|| -> anyhow::Result<()> {
                let key: [u8; 32] = hex::decode(input["key_hex"].as_str().unwrap())?
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("bad key"))?;
                let identity = hex::decode(input["identity_hex"].as_str().unwrap())?;
                let identity = if identity.len() == 16 {
                    uuid::Uuid::from_slice(&identity)?
                } else {
                    uuid::Uuid::parse_str(std::str::from_utf8(&identity)?)?
                };
                let ciphertext = STANDARD.decode(input["ciphertext_base64"].as_str().unwrap())?;
                let plaintext = SecretBox::new(crate::secret_definition::KEY_ID, key)?
                    .decrypt_client_authorization(
                        &identity.to_string(),
                        std::str::from_utf8(&ciphertext)?,
                    )?;
                crate::db::validate_authorization_code(&plaintext)?;
                if let Some(digest) = input["digest_hex"].as_str() {
                    anyhow::ensure!(
                        hex::decode(digest)? == crate::db::token_hash(&plaintext),
                        "digest mismatch"
                    );
                }
                Ok(())
            })()
            .is_ok();
            assert_eq!(
                valid,
                case["valid"].as_bool().unwrap(),
                "runtime differs from installation case {}",
                case["name"]
            );
        }
    }
}
