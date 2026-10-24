//! Administrator-controlled runtime configuration, read without source writes.

use super::{ConfigError, ConfigSource, MAX_CONFIG_BYTES, Reason};
use rustix::fs::{AtFlags, Mode, OFlags, open, openat, statat};
use std::{
    fs::{File, Metadata},
    io::Read,
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    path::{Component, Path},
};

fn error(reason: Reason) -> ConfigError {
    ConfigError::new(reason, "", ConfigSource::File)
}

fn io_error(cause: rustix::io::Errno) -> ConfigError {
    error(match cause {
        rustix::io::Errno::NOENT => Reason::MissingFile,
        rustix::io::Errno::LOOP | rustix::io::Errno::NOTDIR => Reason::UnsafeFile,
        _ => Reason::ReadFailed,
    })
}

fn directory_stamp(metadata: &Metadata) -> (u64, u64, u32, u32, u32) {
    (
        metadata.dev(),
        metadata.ino(),
        metadata.uid(),
        metadata.gid(),
        metadata.mode(),
    )
}

fn file_stamp(metadata: &Metadata) -> (u64, u64, u64, i64, i64, i64, i64, u32, u32, u32, u64) {
    (
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.ctime(),
        metadata.ctime_nsec(),
        metadata.uid(),
        metadata.gid(),
        metadata.mode(),
        metadata.nlink(),
    )
}

fn no_additional_acl(file: &File, directory: bool) -> Result<(), ConfigError> {
    // O_PATH keeps each physical directory without requiring directory listing
    // permission. /proc/self/fd selects that held inode, never a caller path.
    let physical = format!("/proc/self/fd/{}", file.as_raw_fd());
    for attribute in ["system.posix_acl_access", "system.posix_acl_default"] {
        if attribute.ends_with("default") && !directory {
            continue;
        }
        match rustix::fs::getxattr(&physical, attribute, &mut [0u8; 1]) {
            Err(rustix::io::Errno::NODATA | rustix::io::Errno::NOTSUP) => (),
            Ok(_) | Err(rustix::io::Errno::RANGE) => return Err(error(Reason::UnsafeFile)),
            Err(_) => return Err(error(Reason::ReadFailed)),
        }
    }
    Ok(())
}

/// Read administrator-controlled runtime configuration, at most 1 MiB.
///
/// The physical parent is root-owned mode 0700 or 0750; the regular, single-link
/// file is root-owned mode 0600 or 0640 and has the same group as its parent.
/// Ancestors are root-owned and cannot be written by other identities, except
/// root-owned sticky system directories. Links, extended access/default ACLs,
/// unsafe modes, and identity or content changes during inspection are rejected.
///
/// `allow_development_owner` additionally accepts a process-owned 0700 parent
/// and 0600 file. A consumer may set it only for an explicitly unbound debug
/// build. This function never creates files, locks, directories, or permissions.
pub fn read_managed_file(
    path: &Path,
    allow_development_owner: bool,
) -> Result<Vec<u8>, ConfigError> {
    if !path.is_absolute() || path.as_os_str().len() > 16 * 1024 {
        return Err(error(Reason::UnsafeFile));
    }
    let components = path.components().collect::<Vec<_>>();
    if components.len() < 2
        || components.len() > 256
        || components
            .iter()
            .skip(1)
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(error(Reason::UnsafeFile));
    }
    let caller = rustix::process::geteuid().as_raw();
    let flags = OFlags::PATH | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let root = File::from(open("/", flags, Mode::empty()).map_err(io_error)?);
    let mut directories = vec![(root, None)];
    for component in &components[1..components.len() - 1] {
        let Component::Normal(name) = component else {
            unreachable!()
        };
        let parent = &directories.last().expect("root descriptor exists").0;
        let next = File::from(openat(parent, *name, flags, Mode::empty()).map_err(io_error)?);
        directories.push((next, Some(*name)));
    }
    let mut stamps = Vec::with_capacity(directories.len());
    for (index, (directory, _)) in directories.iter().enumerate() {
        let metadata = directory
            .metadata()
            .map_err(|_| error(Reason::ReadFailed))?;
        let mode = metadata.mode() & 0o7777;
        let owner_allowed =
            metadata.uid() == 0 || (allow_development_owner && metadata.uid() == caller);
        let sticky_root = metadata.uid() == 0 && mode & 0o1000 != 0;
        if !metadata.is_dir() || !owner_allowed || (mode & 0o022 != 0 && !sticky_root) {
            return Err(error(Reason::UnsafeFile));
        }
        if index + 1 == directories.len() {
            let managed = metadata.uid() == 0 && matches!(mode, 0o700 | 0o750);
            let development = allow_development_owner && metadata.uid() == caller && mode == 0o700;
            if !managed && !development {
                return Err(error(Reason::UnsafeFile));
            }
            no_additional_acl(directory, true)?;
        }
        stamps.push(directory_stamp(&metadata));
    }
    let directory = &directories.last().expect("root descriptor exists").0;
    let Component::Normal(name) = components.last().expect("file component exists") else {
        unreachable!()
    };
    let mut file = File::from(
        openat(
            directory,
            *name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(io_error)?,
    );
    let before = file.metadata().map_err(|_| error(Reason::ReadFailed))?;
    let parent = stamps.last().expect("parent stamp exists");
    let mode = before.mode() & 0o7777;
    let managed = parent.2 == 0 && before.uid() == 0 && matches!(mode, 0o600 | 0o640);
    let development =
        allow_development_owner && parent.2 == caller && before.uid() == caller && mode == 0o600;
    if !before.is_file()
        || before.nlink() != 1
        || before.gid() != parent.3
        || (!managed && !development)
    {
        return Err(error(Reason::UnsafeFile));
    }
    no_additional_acl(&file, false)?;
    if before.len() > MAX_CONFIG_BYTES as u64 {
        return Err(error(Reason::LimitExceeded));
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_CONFIG_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| error(Reason::ReadFailed))?;
    if bytes.len() > MAX_CONFIG_BYTES {
        return Err(error(Reason::LimitExceeded));
    }
    let after = file.metadata().map_err(|_| error(Reason::ReadFailed))?;
    let named = statat(directory, *name, AtFlags::SYMLINK_NOFOLLOW).map_err(io_error)?;
    if file_stamp(&before) != file_stamp(&after)
        || named.st_dev != before.dev()
        || named.st_ino != before.ino()
        || named.st_mode != before.mode()
        || named.st_uid != before.uid()
        || named.st_gid != before.gid()
        || named.st_nlink != before.nlink()
        || u64::try_from(named.st_size).ok() != Some(before.len())
        || named.st_mtime != before.mtime()
        || i64::try_from(named.st_mtime_nsec).ok() != Some(before.mtime_nsec())
        || named.st_ctime != before.ctime()
        || i64::try_from(named.st_ctime_nsec).ok() != Some(before.ctime_nsec())
        || bytes.len() as u64 != before.len()
    {
        return Err(error(Reason::SourceChanged));
    }
    no_additional_acl(&file, false)?;
    no_additional_acl(directory, true)?;
    for (index, (held, name)) in directories.iter().enumerate() {
        let now = held.metadata().map_err(|_| error(Reason::ReadFailed))?;
        let named = if let Some(name) = name {
            statat(&directories[index - 1].0, *name, AtFlags::SYMLINK_NOFOLLOW).map_err(io_error)?
        } else {
            rustix::fs::stat("/").map_err(io_error)?
        };
        let named_stamp = (
            named.st_dev,
            named.st_ino,
            named.st_uid,
            named.st_gid,
            named.st_mode,
        );
        if directory_stamp(&now) != stamps[index] || named_stamp != stamps[index] {
            return Err(error(Reason::SourceChanged));
        }
    }
    Ok(bytes)
}
