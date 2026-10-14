//! Current persistent envelope definition used by runtime and build-time material export.
use xcss::secret_envelope::EnvelopeDomain;

pub(crate) const PREFIX: &str = "xscs:sgev1:";
pub(crate) const AAD_FORMAT: &[u8] = b"xscs:aes-256-gcm:aad:v1";
pub(crate) const OPERATION_REQUEST_DOMAIN: &[u8] = b"operation-request";
pub(crate) const OPERATION_REQUEST_FIELD: &[u8] = b"request_ciphertext";
pub(crate) const HKDF_SALT: &[u8] = b"xscs:credential-master-key:hkdf-sha256:v1";
pub(crate) const REQUEST_FINGERPRINT_INFO: &[u8] =
    b"xscs:operation-request-fingerprint:hmac-sha256:v1";
pub(crate) const IDEMPOTENCY_KEY_HASH_INFO: &[u8] =
    b"xscs:operation-idempotency-key-hash:hmac-sha256:v1";

pub(crate) struct OperationRequestEnvelope;
impl EnvelopeDomain for OperationRequestEnvelope {
    const DOMAIN: &'static [u8] = b"xscs/operation-request";
    const REVISION: u16 = 1;
}

pub(crate) struct ClientAuthorizationEnvelope;
impl EnvelopeDomain for ClientAuthorizationEnvelope {
    const DOMAIN: &'static [u8] = b"xscs/client-authorization";
    const REVISION: u16 = 1;
}
