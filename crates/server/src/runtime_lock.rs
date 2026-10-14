#[cfg(target_os = "linux")]
mod platform {
    use std::{
        ffi::{OsStr, OsString},
        fs::File,
        os::fd::AsRawFd,
        path::{Path, PathBuf},
    };

    use anyhow::{Context, ensure};
    use rustix::{
        fs::{FileType, FlockOperation, Mode, OFlags, ResolveFlags, flock, fstat, open, openat2},
        io::Errno,
    };

    /// Locks held from before SQLite is opened until every worker has stopped.
    pub struct ApplicationLock {
        location: DatabaseLocation,
        _instance: File,
    }

    impl ApplicationLock {
        pub fn acquire(database_url: &str) -> anyhow::Result<Self> {
            let location = DatabaseLocation::resolve(database_url)?;
            let instance = location
                .acquire(".xscs.instance.lock", LockKind::Exclusive)
                .context("another xscs process already owns this SQLite database")?;
            Ok(Self {
                location,
                _instance: instance,
            })
        }

        /// SQLite URL rooted at the already verified directory descriptor.
        pub fn database_url(&self) -> String {
            self.location.database_url()
        }

        pub fn database_path(&self) -> PathBuf {
            self.location.database_path()
        }
    }

    enum LockKind {
        Exclusive,
    }

    struct DatabaseLocation {
        parent: File,
        database_name: OsString,
    }

    impl DatabaseLocation {
        fn resolve(database_url: &str) -> anyhow::Result<Self> {
            let database = crate::database_schema::database_path(database_url)?;
            let parent = database
                .parent()
                .context("SQLite database path must have a parent")?;
            let database_name = database
                .file_name()
                .context("SQLite database path must name a file")?
                .to_os_string();
            let filesystem_root = open(
                "/",
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                Mode::empty(),
            )?;
            let relative_parent = parent
                .strip_prefix("/")
                .context("SQLite database parent must be absolute")?;
            let relative_parent = if relative_parent.as_os_str().is_empty() {
                Path::new(".")
            } else {
                relative_parent
            };
            let resolve =
                ResolveFlags::BENEATH | ResolveFlags::NO_MAGICLINKS | ResolveFlags::NO_SYMLINKS;
            let parent = openat2(
                &filesystem_root,
                relative_parent,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                Mode::empty(),
                resolve,
            )
            .context("open SQLite database parent without following symbolic links")?;
            let parent = File::from(parent);
            validate_database_entry(&parent, &database_name, resolve)?;
            Ok(Self {
                parent,
                database_name,
            })
        }

        fn acquire(&self, suffix: &str, kind: LockKind) -> anyhow::Result<File> {
            let lock_name = lock_name(&self.database_name, suffix);
            let resolve =
                ResolveFlags::BENEATH | ResolveFlags::NO_MAGICLINKS | ResolveFlags::NO_SYMLINKS;
            let fd = openat2(
                &self.parent,
                lock_name,
                OFlags::RDWR
                    | OFlags::CREATE
                    | OFlags::NOFOLLOW
                    | OFlags::NONBLOCK
                    | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
                resolve,
            )
            .context("open xscs database lock without following symbolic links")?;
            let metadata = fstat(&fd)?;
            ensure!(
                FileType::from_raw_mode(metadata.st_mode) == FileType::RegularFile
                    && metadata.st_nlink == 1,
                "xscs database lock must be one regular file"
            );
            let operation = match kind {
                LockKind::Exclusive => FlockOperation::NonBlockingLockExclusive,
            };
            match flock(&fd, operation) {
                Ok(()) => Ok(File::from(fd)),
                Err(Errno::WOULDBLOCK) => anyhow::bail!("database lock is already held"),
                Err(error) => {
                    Err(std::io::Error::from(error)).context("acquire xscs database lock")
                }
            }
        }

        fn database_path(&self) -> PathBuf {
            PathBuf::from(format!("/proc/self/fd/{}", self.parent.as_raw_fd()))
                .join(&self.database_name)
        }

        fn database_url(&self) -> String {
            format!("sqlite://{}", self.database_path().display())
        }
    }

    fn validate_database_entry(
        parent: &File,
        database_name: &OsStr,
        resolve: ResolveFlags,
    ) -> anyhow::Result<()> {
        match openat2(
            parent,
            database_name,
            OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
            resolve,
        ) {
            Ok(fd) => {
                let metadata = fstat(fd)?;
                ensure!(
                    FileType::from_raw_mode(metadata.st_mode) == FileType::RegularFile
                        && metadata.st_nlink == 1,
                    "SQLite database path must be one regular file"
                );
                Ok(())
            }
            Err(Errno::NOENT) => Ok(()),
            Err(error) => Err(std::io::Error::from(error))
                .context("validate SQLite database without following symbolic links"),
        }
    }

    fn lock_name(database_name: &OsStr, suffix: &str) -> OsString {
        let mut name = OsString::from(".");
        name.push(database_name);
        name.push(suffix);
        name
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn database_url(directory: &Path, name: &str) -> String {
            format!("sqlite://{}", directory.join(name).display())
        }

        #[test]
        fn application_lock_is_exclusive_and_released_on_drop() {
            let directory = tempfile::tempdir().unwrap();
            let url = database_url(directory.path(), "app.sqlite3");
            let application = ApplicationLock::acquire(&url).unwrap();
            assert!(ApplicationLock::acquire(&url).is_err());
            drop(application);
            ApplicationLock::acquire(&url).unwrap();
        }

        #[test]
        fn distinct_database_paths_have_independent_locks() {
            let directory = tempfile::tempdir().unwrap();
            let first = database_url(directory.path(), "first.sqlite3");
            let second = database_url(directory.path(), "second.sqlite3");
            let _first = ApplicationLock::acquire(&first).unwrap();
            ApplicationLock::acquire(&second).unwrap();
        }

        #[test]
        fn lexical_aliases_share_one_lock_identity() {
            let directory = tempfile::tempdir().unwrap();
            let direct = database_url(directory.path(), "app.sqlite3");
            let dotted = format!("sqlite://{}/./app.sqlite3", directory.path().display());
            let _direct = ApplicationLock::acquire(&direct).unwrap();
            assert!(ApplicationLock::acquire(&dotted).is_err());
        }

        #[test]
        fn parent_traversal_is_rejected() {
            let directory = tempfile::tempdir().unwrap();
            std::fs::create_dir(directory.path().join("child")).unwrap();
            let url = format!(
                "sqlite://{}/child/../app.sqlite3",
                directory.path().display()
            );
            assert!(ApplicationLock::acquire(&url).is_err());
        }

        #[test]
        fn symbolic_links_and_special_database_files_are_rejected() {
            use std::os::unix::fs::symlink;

            let directory = tempfile::tempdir().unwrap();
            let real = directory.path().join("real");
            std::fs::create_dir(&real).unwrap();
            let linked_parent = directory.path().join("linked-parent");
            symlink(&real, &linked_parent).unwrap();
            assert!(
                ApplicationLock::acquire(&database_url(&linked_parent, "app.sqlite3")).is_err()
            );

            let real_database = real.join("real.sqlite3");
            File::create(&real_database).unwrap();
            let linked_database = real.join("linked.sqlite3");
            symlink(&real_database, &linked_database).unwrap();
            assert!(ApplicationLock::acquire(&database_url(&real, "linked.sqlite3")).is_err());

            let special = real.join("database.directory");
            std::fs::create_dir(&special).unwrap();
            assert!(ApplicationLock::acquire(&database_url(&real, "database.directory")).is_err());
        }

        #[test]
        fn symbolic_and_hard_linked_lock_files_are_rejected() {
            use std::os::unix::fs::symlink;

            let directory = tempfile::tempdir().unwrap();
            let symbolic_url = database_url(directory.path(), "symbolic.sqlite3");
            symlink(
                directory.path().join("outside"),
                directory
                    .path()
                    .join(".symbolic.sqlite3.xscs.instance.lock"),
            )
            .unwrap();
            assert!(ApplicationLock::acquire(&symbolic_url).is_err());

            let lock_target = directory.path().join("lock-target");
            File::create(&lock_target).unwrap();
            std::fs::hard_link(
                &lock_target,
                directory.path().join(".hard.sqlite3.xscs.instance.lock"),
            )
            .unwrap();
            assert!(
                ApplicationLock::acquire(&database_url(directory.path(), "hard.sqlite3")).is_err()
            );
        }

        #[test]
        fn hard_linked_databases_are_rejected() {
            let directory = tempfile::tempdir().unwrap();
            let database = directory.path().join("database.sqlite3");
            File::create(&database).unwrap();
            let alias = directory.path().join("database-hardlink.sqlite3");
            std::fs::hard_link(&database, &alias).unwrap();
            assert!(
                ApplicationLock::acquire(&database_url(directory.path(), "database.sqlite3"))
                    .is_err()
            );
            assert!(
                ApplicationLock::acquire(&database_url(
                    directory.path(),
                    "database-hardlink.sqlite3"
                ))
                .is_err()
            );
        }

        #[tokio::test]
        async fn verified_directory_descriptor_is_the_sqlite_identity_across_restarts() {
            let directory = tempfile::tempdir().unwrap();
            let database = directory.path().join("app.sqlite3");
            let configured_url = database_url(directory.path(), "app.sqlite3");

            let application = ApplicationLock::acquire(&configured_url).unwrap();
            let pool = crate::db::open_or_initialize(&application.database_url())
                .await
                .unwrap();
            sqlx::query(
                "INSERT INTO devices(device_id,name,authorization_code_enc,created_at_micros,updated_at_micros) \
                 VALUES('11111111-1111-4111-8111-111111111111','Locked',?,1,1)",
            )
            .bind("x".repeat(80))
            .execute(&pool)
            .await
            .unwrap();
            let mut connections = Vec::new();
            while connections.len() < pool.size() as usize {
                connections.push(pool.acquire().await.unwrap());
            }
            let closed = pool.close();
            for connection in connections {
                connection.close().await.unwrap();
            }
            closed.await;
            assert!(database.is_file());
            drop(application);

            let restarted = ApplicationLock::acquire(&configured_url).unwrap();
            crate::database_schema::validate_configuration_database(&configured_url).unwrap();
            let reopened =
                crate::database_schema::open_validated_location(&restarted.database_url())
                    .await
                    .unwrap();
            let name: String = sqlx::query_scalar(
                "SELECT name FROM devices WHERE device_id='11111111-1111-4111-8111-111111111111'",
            )
            .fetch_one(&reopened)
            .await
            .unwrap();
            assert_eq!(name, "Locked");
            reopened.close().await;
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod platform {
    use std::path::PathBuf;

    pub struct ApplicationLock;

    impl ApplicationLock {
        pub fn acquire(_: &str) -> anyhow::Result<Self> {
            anyhow::bail!("secure xscs database locks require Linux openat2")
        }

        pub fn database_url(&self) -> String {
            unreachable!()
        }

        pub fn database_path(&self) -> PathBuf {
            unreachable!()
        }
    }
}

pub use platform::ApplicationLock;
