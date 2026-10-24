//! Persistent installation secrets, created explicitly during initialization.
use crate::fs_safety::{AtomicFile, EntryName, PrivateDirectory};
use serde::{Serialize, de::DeserializeOwned};
use std::path::Path;
use zeroize::Zeroizing;

const FILE: &str = "secrets.json";
const MAX_BYTES: usize = 16 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Filesystem(#[from] crate::fs_safety::Error),
    #[error("installation secrets document is invalid")]
    InvalidDocument,
    #[error("operating system randomness is unavailable")]
    RandomnessUnavailable,
}

pub fn generate_key() -> Result<[u8; 32], Error> {
    let mut key = [0; 32];
    getrandom::fill(&mut key).map_err(|_| Error::RandomnessUnavailable)?;
    Ok(key)
}

/// Create once in an existing private data directory. Never replaces a key file.
pub fn initialize<T: Serialize>(data_dir: &Path, value: &T) -> Result<(), Error> {
    let bytes = Zeroizing::new(serde_json::to_vec(value).map_err(|_| Error::InvalidDocument)?);
    if bytes.len() > MAX_BYTES {
        return Err(Error::InvalidDocument);
    }
    let directory = PrivateDirectory::open_existing(data_dir)?;
    AtomicFile::create(&directory, &EntryName::new(FILE)?, &bytes)?;
    Ok(())
}

/// Read existing installation secrets without creating or changing any state.
pub fn load<T: DeserializeOwned>(data_dir: &Path) -> Result<T, Error> {
    let directory = PrivateDirectory::open_existing(data_dir)?;
    let bytes = Zeroizing::new(directory.read_bounded(&EntryName::new(FILE)?, MAX_BYTES)?);
    serde_json::from_slice(&bytes).map_err(|_| Error::InvalidDocument)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Secrets {
        key: [u8; 32],
    }

    #[test]
    fn initialization_is_persistent_private_and_no_clobber() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data");
        let directory = PrivateDirectory::create(&path).unwrap();
        let original = Secrets {
            key: generate_key().unwrap(),
        };
        let other = Secrets {
            key: generate_key().unwrap(),
        };
        assert_ne!(original, other);
        assert_ne!(original.key, [0; 32]);
        initialize(&path, &original).unwrap();
        assert!(initialize(&path, &other).is_err());
        assert_eq!(load::<Secrets>(&path).unwrap(), original);
        assert_eq!(
            std::fs::metadata(path.join(FILE))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o700
        );
        AtomicFile::replace(
            &directory,
            &crate::fs_safety::RelativePath::new(FILE).unwrap(),
            b"{}",
        )
        .unwrap();
        assert!(load::<Secrets>(&path).is_err());
    }

    #[test]
    fn loading_missing_secrets_never_initializes() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("missing");
        assert!(load::<Secrets>(&path).is_err());
        assert!(!path.exists());
        PrivateDirectory::create(&path).unwrap();
        assert!(load::<Secrets>(&path).is_err());
        assert!(!path.join(FILE).exists());
    }
}
