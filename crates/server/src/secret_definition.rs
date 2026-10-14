//! Current secret identity, shape and key encoding shared by runtime and material export.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
pub(crate) const VERSION: u32 = 1;
pub(crate) const KEY_ID: &str = "primary";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SecretsDocument {
    pub(crate) version: u32,
    pub(crate) credential_key_id: String,
    pub(crate) credential_key: String,
}
impl SecretsDocument {
    pub(crate) fn key(&self) -> anyhow::Result<[u8; 32]> {
        anyhow::ensure!(
            self.version == VERSION && self.credential_key_id == KEY_ID,
            "invalid secrets document identity"
        );
        let key: [u8; 32] = STANDARD
            .decode(&self.credential_key)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("credential key must decode to exactly 32 bytes"))?;
        anyhow::ensure!(
            STANDARD.encode(key) == self.credential_key,
            "credential key must use canonical base64"
        );
        Ok(key)
    }
}
