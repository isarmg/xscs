use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::Read,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    database_schema::{APPLICATION, APPLICATION_VERSION},
    release_contract::{BinaryIdentity, current_json},
};

pub const RELEASE_MANIFEST_FORMAT: &str = "xscs-files-v1";
pub const RELEASE_MANIFEST_NAME: &str = "RELEASE-MANIFEST.json";
const MAX_MANIFEST_BYTES: u64 = 8 * 1024 * 1024;
const MAX_ENTRIES: usize = 10_000;
const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_RELEASE_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleaseManifest {
    manifest_format: String,
    application: String,
    version: String,
    source_revision: String,
    binary_identity_sha256: String,
    entries: Vec<ReleaseEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ReleaseEntry {
    Directory {
        path: String,
        mode: String,
    },
    File {
        path: String,
        mode: String,
        size: u64,
        sha256: String,
    },
}

impl ReleaseEntry {
    fn path(&self) -> &str {
        match self {
            Self::Directory { path, .. } | Self::File { path, .. } => path,
        }
    }

    fn mode(&self) -> &str {
        match self {
            Self::Directory { mode, .. } | Self::File { mode, .. } => mode,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReleaseVerification {
    pub status: &'static str,
    pub application: &'static str,
    pub version: &'static str,
    pub source_revision: String,
    pub files: usize,
    pub bytes: u64,
}

pub fn verify_release(root: &Path) -> anyhow::Result<ReleaseVerification> {
    verify_release_with_options(root, true, true)
}

fn verify_release_with_options(
    root: &Path,
    require_current_executable: bool,
    require_bound_source: bool,
) -> anyhow::Result<ReleaseVerification> {
    crate::web_assets::verify()?;
    validate_release_root(root)?;
    let manifest_path = root.join(RELEASE_MANIFEST_NAME);
    let manifest_metadata =
        fs::symlink_metadata(&manifest_path).context("release manifest is missing")?;
    ensure_regular_metadata(&manifest_metadata, "release manifest")?;
    ensure!(
        permission_mode(&manifest_metadata) == 0o444,
        "release manifest must have mode 0444"
    );
    let manifest_bytes = read_verified_file(&manifest_path, MAX_MANIFEST_BYTES)?;
    let manifest: ReleaseManifest =
        serde_json::from_slice(&manifest_bytes).context("release manifest must be strict JSON")?;

    let identity = BinaryIdentity::current()?;
    ensure!(
        !require_bound_source || identity.is_release_bound(),
        "release binary has no exact source revision binding"
    );
    ensure!(
        manifest.manifest_format == RELEASE_MANIFEST_FORMAT
            && manifest.application == APPLICATION
            && manifest.version == APPLICATION_VERSION
            && manifest.source_revision == identity.source_revision,
        "release manifest does not match the exact current binary identity"
    );
    validate_sha256(&manifest.binary_identity_sha256)?;
    ensure!(
        manifest.binary_identity_sha256 == encode_hex(&Sha256::digest(current_json()?.as_bytes())),
        "release manifest binary identity fingerprint mismatch"
    );
    ensure!(
        !require_bound_source || is_full_git_revision(&manifest.source_revision),
        "release manifest source revision is not a full lowercase Git commit"
    );
    ensure!(
        !manifest.entries.is_empty() && manifest.entries.len() <= MAX_ENTRIES,
        "release manifest entry count is invalid"
    );

    if require_current_executable {
        let current = fs::canonicalize(std::env::current_exe()?)?;
        let expected = fs::canonicalize(root.join("bin/xscs"))?;
        ensure!(
            current == expected,
            "release must be verified by its own xscs binary"
        );
    }

    let expected = parse_entries(&manifest.entries)?;
    validate_required_layout(&expected)?;
    let mut seen = HashSet::with_capacity(expected.len());
    let mut counters = Counters::default();
    validate_directory(root, Path::new(""), &expected, &mut seen, &mut counters)?;
    ensure!(
        read_verified_file(&root.join("web-assets.json"), MAX_MANIFEST_BYTES)?
            == crate::web_assets::MANIFEST.as_bytes(),
        "release Web inventory does not match the bytes compiled into this binary"
    );
    ensure!(
        seen.len() == expected.len(),
        "release is missing one or more manifest entries"
    );

    Ok(ReleaseVerification {
        status: "ok",
        application: APPLICATION,
        version: APPLICATION_VERSION,
        source_revision: manifest.source_revision,
        files: counters.files,
        bytes: counters.bytes,
    })
}

fn validate_release_root(root: &Path) -> anyhow::Result<()> {
    ensure!(root.is_absolute(), "release root must be absolute");
    ensure!(
        root.components()
            .all(|component| { matches!(component, Component::RootDir | Component::Normal(_)) }),
        "release root must be a normalized absolute path"
    );
    ensure!(
        root.file_name().and_then(|value| value.to_str()) == Some(APPLICATION_VERSION)
            && root
                .parent()
                .and_then(Path::file_name)
                .and_then(|value| value.to_str())
                == Some("releases"),
        "release root must be releases/{APPLICATION_VERSION}"
    );
    ensure!(
        fs::canonicalize(root)? == root,
        "release root and every parent component must be real directories"
    );
    let metadata = fs::symlink_metadata(root)?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "release root must be a real directory"
    );
    ensure!(
        permission_mode(&metadata) == 0o555,
        "release root must have mode 0555"
    );
    Ok(())
}

fn parse_entries(entries: &[ReleaseEntry]) -> anyhow::Result<HashMap<PathBuf, ReleaseEntry>> {
    let mut expected = HashMap::with_capacity(entries.len());
    let mut previous = None;
    for entry in entries {
        let path = validated_relative_path(entry.path())?;
        ensure!(
            entry.path() != RELEASE_MANIFEST_NAME,
            "release manifest must not list itself"
        );
        if let Some(previous) = previous {
            ensure!(
                previous < entry.path(),
                "release entries are not strictly sorted"
            );
        }
        previous = Some(entry.path());
        let mode = parse_mode(entry.mode())?;
        ensure!(mode & 0o222 == 0, "release entries must be read-only");
        match entry {
            ReleaseEntry::Directory { .. } => {
                ensure!(mode == 0o555, "release directories must have mode 0555")
            }
            ReleaseEntry::File { size, sha256, .. } => {
                ensure!(*size <= MAX_FILE_BYTES, "release file exceeds size limit");
                validate_sha256(sha256)?;
            }
        }
        ensure!(
            expected.insert(path, entry.clone()).is_none(),
            "release manifest contains a duplicate path"
        );
    }
    Ok(expected)
}

fn validate_required_layout(expected: &HashMap<PathBuf, ReleaseEntry>) -> anyhow::Result<()> {
    const DIRECTORIES: [&str; 3] = ["bin", "systemd", "config"];
    const FILES: [&str; 6] = [
        "README.md",
        "install.sh",
        "config/config.json.example",
        "bin/xscs",
        "systemd/xscs.service",
        "web-assets.json",
    ];
    ensure!(
        expected.len() == DIRECTORIES.len() + FILES.len(),
        "release must contain only the exact embedded-Web layout"
    );
    for directory in DIRECTORIES {
        ensure!(
            matches!(expected.get(Path::new(directory)), Some(ReleaseEntry::Directory { mode, .. }) if mode == "0555"),
            "release is missing required directory {directory}"
        );
    }
    for file in FILES {
        let expected_mode = if file.starts_with("bin/") || file == "install.sh" {
            "0555"
        } else {
            "0444"
        };
        ensure!(
            matches!(expected.get(Path::new(file)), Some(ReleaseEntry::File { mode, .. }) if mode == expected_mode),
            "release is missing required file {file}"
        );
    }
    Ok(())
}

#[derive(Default)]
struct Counters {
    entries: usize,
    files: usize,
    bytes: u64,
}

fn validate_directory(
    root: &Path,
    relative: &Path,
    expected: &HashMap<PathBuf, ReleaseEntry>,
    seen: &mut HashSet<PathBuf>,
    counters: &mut Counters,
) -> anyhow::Result<()> {
    let path = root.join(relative);
    let before = fs::symlink_metadata(&path)?;
    ensure!(
        before.is_dir() && !before.file_type().is_symlink(),
        "release contains an invalid directory"
    );
    if !relative.as_os_str().is_empty() {
        let Some(ReleaseEntry::Directory { mode, .. }) = expected.get(relative) else {
            bail!(
                "release contains an unexpected directory: {}",
                relative.display()
            );
        };
        ensure!(
            permission_mode(&before) == parse_mode(mode)?,
            "release directory mode mismatch: {}",
            relative.display()
        );
        ensure!(
            seen.insert(relative.to_path_buf()),
            "duplicate release path"
        );
    }

    let mut children = fs::read_dir(&path)?.collect::<std::io::Result<Vec<_>>>()?;
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        let name = child
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("release contains a non-UTF-8 name"))?;
        validate_name(&name)?;
        let child_relative = relative.join(&name);
        if child_relative == Path::new(RELEASE_MANIFEST_NAME) {
            continue;
        }
        counters.entries += 1;
        ensure!(
            counters.entries <= MAX_ENTRIES,
            "release contains too many entries"
        );
        let metadata = fs::symlink_metadata(child.path())?;
        ensure!(
            !metadata.file_type().is_symlink(),
            "release contains a symbolic link"
        );
        if metadata.is_dir() {
            validate_directory(root, &child_relative, expected, seen, counters)?;
            continue;
        }
        ensure!(metadata.is_file(), "release contains a special file");
        let Some(ReleaseEntry::File {
            mode, size, sha256, ..
        }) = expected.get(&child_relative)
        else {
            bail!(
                "release contains an unexpected file: {}",
                child_relative.display()
            );
        };
        ensure_regular_metadata(&metadata, "release file")?;
        ensure!(
            permission_mode(&metadata) == parse_mode(mode)?,
            "release file mode mismatch: {}",
            child_relative.display()
        );
        ensure!(
            metadata.len() == *size,
            "release file size mismatch: {}",
            child_relative.display()
        );
        let bytes = read_verified_file(&child.path(), MAX_FILE_BYTES)?;
        ensure!(
            encode_hex(&Sha256::digest(&bytes)) == *sha256,
            "release file digest mismatch: {}",
            child_relative.display()
        );
        counters.files += 1;
        counters.bytes = counters
            .bytes
            .checked_add(*size)
            .context("release byte count overflow")?;
        ensure!(
            counters.bytes <= MAX_RELEASE_BYTES,
            "release exceeds total size limit"
        );
        ensure!(seen.insert(child_relative), "duplicate release path");
    }
    let after = fs::symlink_metadata(&path)?;
    ensure!(
        before.dev() == after.dev() && before.ino() == after.ino(),
        "release directory changed during verification"
    );
    Ok(())
}

fn read_verified_file(path: &Path, maximum: u64) -> anyhow::Result<Vec<u8>> {
    let path_metadata = fs::symlink_metadata(path)?;
    ensure_regular_metadata(&path_metadata, "release file")?;
    ensure!(path_metadata.len() <= maximum, "release file is too large");
    let mut file = File::open(path)?;
    let opened = file.metadata()?;
    ensure_regular_metadata(&opened, "opened release file")?;
    ensure!(
        opened.dev() == path_metadata.dev() && opened.ino() == path_metadata.ino(),
        "release file changed while it was opened"
    );
    let mut bytes = Vec::with_capacity(opened.len() as usize);
    file.by_ref().take(maximum + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= maximum, "release file is too large");
    let after = file.metadata()?;
    ensure!(
        opened.dev() == after.dev()
            && opened.ino() == after.ino()
            && opened.len() == after.len()
            && after.nlink() == 1,
        "release file changed while it was read"
    );
    Ok(bytes)
}

fn ensure_regular_metadata(metadata: &fs::Metadata, label: &str) -> anyhow::Result<()> {
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "{label} must be a regular file"
    );
    ensure!(
        metadata.nlink() == 1,
        "{label} must have exactly one hard link"
    );
    Ok(())
}

fn validated_relative_path(value: &str) -> anyhow::Result<PathBuf> {
    ensure!(
        !value.is_empty() && value.len() <= 1024,
        "invalid release path"
    );
    let path = PathBuf::from(value);
    ensure!(!path.is_absolute(), "release paths must be relative");
    ensure!(
        path.components()
            .all(|component| matches!(component, Component::Normal(_))),
        "release path contains an unsafe component"
    );
    for component in path.components() {
        if let Component::Normal(name) = component {
            validate_name(name.to_str().context("release path is not UTF-8")?)?;
        }
    }
    ensure!(
        path.to_str() == Some(value),
        "release path is not canonical"
    );
    Ok(path)
}

fn validate_name(value: &str) -> anyhow::Result<()> {
    ensure!(
        !value.is_empty()
            && value != "."
            && value != ".."
            && value
                .bytes()
                .all(|byte| { byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-') }),
        "release contains a non-portable name"
    );
    Ok(())
}

fn permission_mode(metadata: &fs::Metadata) -> u32 {
    metadata.permissions().mode() & 0o7777
}

fn parse_mode(value: &str) -> anyhow::Result<u32> {
    ensure!(
        value.len() == 4 && value.bytes().all(|byte| matches!(byte, b'0'..=b'7')),
        "release mode must be four octal digits"
    );
    u32::from_str_radix(value, 8).context("release mode is invalid")
}

fn validate_sha256(value: &str) -> anyhow::Result<()> {
    ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')),
        "SHA-256 values must contain 64 lowercase hexadecimal digits"
    );
    Ok(())
}

fn is_full_git_revision(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    struct Fixture {
        _temp: TempDir,
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("releases").join(APPLICATION_VERSION);
            for directory in ["bin", "systemd", "config"] {
                fs::create_dir_all(root.join(directory)).unwrap();
            }
            let mut entries = Vec::new();
            for directory in ["bin", "systemd", "config"] {
                entries.push(ReleaseEntry::Directory {
                    path: directory.to_owned(),
                    mode: "0555".to_owned(),
                });
            }
            for (path, bytes, mode) in [
                ("README.md", b"current release".as_slice(), "0444"),
                ("install.sh", b"#!/bin/sh".as_slice(), "0555"),
                ("config/config.json.example", b"{}".as_slice(), "0444"),
                ("bin/xscs", b"current binary".as_slice(), "0555"),
                ("systemd/xscs.service", b"[Service]".as_slice(), "0444"),
                (
                    "web-assets.json",
                    crate::web_assets::MANIFEST.as_bytes(),
                    "0444",
                ),
            ] {
                fs::write(root.join(path), bytes).unwrap();
                fs::set_permissions(
                    root.join(path),
                    fs::Permissions::from_mode(u32::from_str_radix(mode, 8).unwrap()),
                )
                .unwrap();
                entries.push(ReleaseEntry::File {
                    path: path.to_owned(),
                    mode: mode.to_owned(),
                    size: bytes.len() as u64,
                    sha256: encode_hex(&Sha256::digest(bytes)),
                });
            }
            for directory in ["bin", "systemd", "config"] {
                fs::set_permissions(root.join(directory), fs::Permissions::from_mode(0o555))
                    .unwrap();
            }
            entries.sort_by(|left, right| left.path().cmp(right.path()));
            let manifest = ReleaseManifest {
                manifest_format: RELEASE_MANIFEST_FORMAT.to_owned(),
                application: APPLICATION.to_owned(),
                version: APPLICATION_VERSION.to_owned(),
                source_revision: BinaryIdentity::current().unwrap().source_revision,
                binary_identity_sha256: encode_hex(&Sha256::digest(
                    current_json().unwrap().as_bytes(),
                )),
                entries,
            };
            fs::write(
                root.join(RELEASE_MANIFEST_NAME),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
            fs::set_permissions(
                root.join(RELEASE_MANIFEST_NAME),
                fs::Permissions::from_mode(0o444),
            )
            .unwrap();
            fs::set_permissions(&root, fs::Permissions::from_mode(0o555)).unwrap();
            Self { _temp: temp, root }
        }
        fn make_writable(&self) {
            if !self.root.exists() {
                return;
            }
            fs::set_permissions(&self.root, fs::Permissions::from_mode(0o755)).unwrap();
            for directory in ["bin", "systemd", "config"] {
                fs::set_permissions(self.root.join(directory), fs::Permissions::from_mode(0o755))
                    .unwrap();
            }
        }
        fn manifest(&self) -> ReleaseManifest {
            serde_json::from_slice(&fs::read(self.root.join(RELEASE_MANIFEST_NAME)).unwrap())
                .unwrap()
        }
        fn write_manifest(&self, manifest: &ReleaseManifest) {
            let path = self.root.join(RELEASE_MANIFEST_NAME);
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
            fs::write(&path, serde_json::to_vec(manifest).unwrap()).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            self.make_writable();
        }
    }

    #[test]
    fn recomputing_external_manifest_does_not_authorize_changed_web_inventory() {
        let fixture = Fixture::new();
        let path = fixture.root.join("web-assets.json");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        fs::write(&path, b"{}").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let mut manifest = fixture.manifest();
        for entry in &mut manifest.entries {
            if let ReleaseEntry::File {
                path, size, sha256, ..
            } = entry
                && path == "web-assets.json"
            {
                *size = 2;
                *sha256 = encode_hex(&Sha256::digest(b"{}"));
            }
        }
        fixture.write_manifest(&manifest);
        assert!(verify_release_with_options(&fixture.root, false, false).is_err());
    }

    #[test]
    fn exact_current_tree_verifies_and_tampering_fails() {
        let fixture = Fixture::new();
        let report = verify_release_with_options(&fixture.root, false, false).unwrap();
        assert_eq!(report.application, APPLICATION);
        assert_eq!(report.files, 6);

        fs::set_permissions(
            fixture.root.join("web-assets.json"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        fs::write(fixture.root.join("web-assets.json"), b"tampered").unwrap();
        fs::set_permissions(
            fixture.root.join("web-assets.json"),
            fs::Permissions::from_mode(0o444),
        )
        .unwrap();
        assert!(verify_release_with_options(&fixture.root, false, false).is_err());
    }

    #[test]
    fn extra_files_unknown_fields_and_unbound_release_identity_fail_closed() {
        let fixture = Fixture::new();
        fixture.make_writable();
        fs::write(fixture.root.join("unexpected-launcher.sh"), b"unexpected").unwrap();
        fs::set_permissions(
            fixture.root.join("unexpected-launcher.sh"),
            fs::Permissions::from_mode(0o444),
        )
        .unwrap();
        fs::set_permissions(&fixture.root, fs::Permissions::from_mode(0o555)).unwrap();
        assert!(verify_release_with_options(&fixture.root, false, false).is_err());

        fixture.make_writable();
        fs::remove_file(fixture.root.join("unexpected-launcher.sh")).unwrap();
        let manifest_path = fixture.root.join(RELEASE_MANIFEST_NAME);
        fs::set_permissions(&manifest_path, fs::Permissions::from_mode(0o644)).unwrap();
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["unknown_extension"] = serde_json::json!(true);
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        fs::set_permissions(&manifest_path, fs::Permissions::from_mode(0o444)).unwrap();
        assert!(verify_release_with_options(&fixture.root, false, false).is_err());

        let fresh = Fixture::new();
        if BinaryIdentity::current().unwrap().source_revision == "unbound" {
            assert!(verify_release_with_options(&fresh.root, false, true).is_err());
        }
    }
}
