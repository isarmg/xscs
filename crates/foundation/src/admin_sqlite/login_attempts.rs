//! Bounded registry of physical administrator-database admission scopes.

use crate::admin_core::{AuthenticationState, AuthenticationStateError, LOGIN_ATTEMPTS_SCOPES_MAX};
use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};
use std::{
    collections::HashMap,
    fs::File,
    os::unix::fs::MetadataExt,
    sync::{Arc, Mutex, OnceLock, Weak},
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Scope {
    File { device: u64, inode: u64 },
    Memory(usize),
}

#[derive(Debug)]
struct Entry {
    state: Arc<AuthenticationState>,
    // Pin memory-scope pointer identity too: an allocator cannot reuse the
    // options address while its unexpired registry entry still exists.
    owner: Weak<SqliteConnectOptions>,
    _memory_identity_pin: Option<Arc<SqliteConnectOptions>>,
    // Keeping the verified main file open prevents an unlinked database's
    // device/inode from being recycled into a different installation scope.
    // O_PATH is essential: closing an ordinary descriptor for the same inode
    // would release this process's SQLite POSIX byte-range locks.
    _database_identity_pin: Option<File>,
}

#[derive(Debug, Default)]
struct Registry {
    entries: HashMap<Scope, Entry>,
}

impl Registry {
    fn acquire(
        &mut self,
        scope: Scope,
        owner: &Arc<SqliteConnectOptions>,
        database_identity_pin: Option<File>,
    ) -> Result<Arc<AuthenticationState>, AuthenticationStateError> {
        if let Some(entry) = self.entries.get_mut(&scope) {
            entry.owner = Arc::downgrade(owner);
            return Ok(entry.state.clone());
        }
        // Never evict a live counter or an unexpired window: recreating a
        // service/pool must not reset the installation's admitted attempts.
        self.entries.retain(|_, entry| {
            entry.owner.strong_count() > usize::from(entry._memory_identity_pin.is_some())
                || Arc::strong_count(&entry.state) != 1
                || !entry.state.can_retire()
        });
        if self.entries.len() == LOGIN_ATTEMPTS_SCOPES_MAX {
            return Err(AuthenticationStateError::Unavailable);
        }
        let state = Arc::new(AuthenticationState::new());
        self.entries.insert(
            scope,
            Entry {
                state: state.clone(),
                owner: Arc::downgrade(owner),
                _memory_identity_pin: matches!(scope, Scope::Memory(_)).then(|| owner.clone()),
                _database_identity_pin: database_identity_pin,
            },
        );
        Ok(state)
    }
}

pub(super) async fn state_for_pool(
    pool: &SqlitePool,
) -> Result<Arc<AuthenticationState>, AuthenticationStateError> {
    let filename: String =
        sqlx::query_scalar("SELECT file FROM pragma_database_list WHERE name='main'")
            .fetch_one(pool)
            .await
            .map_err(|_| AuthenticationStateError::Unavailable)?;
    let owner = pool.connect_options();
    let (scope, database_identity_pin) = if filename.is_empty() {
        (Scope::Memory(Arc::as_ptr(&owner) as usize), None)
    } else {
        let file = rustix::fs::open(
            &filename,
            rustix::fs::OFlags::PATH | rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map(File::from)
        .map_err(|_| AuthenticationStateError::Unavailable)?;
        let metadata = file
            .metadata()
            .map_err(|_| AuthenticationStateError::Unavailable)?;
        let named = std::fs::symlink_metadata(&filename)
            .map_err(|_| AuthenticationStateError::Unavailable)?;
        if !metadata.is_file()
            || !named.is_file()
            || (metadata.dev(), metadata.ino()) != (named.dev(), named.ino())
        {
            return Err(AuthenticationStateError::Unavailable);
        }
        (
            Scope::File {
                device: metadata.dev(),
                inode: metadata.ino(),
            },
            Some(file),
        )
    };
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY
        .get_or_init(|| Mutex::new(Registry::default()))
        .lock()
        .map_err(|_| AuthenticationStateError::Unavailable)?
        .acquire(scope, &owner, database_identity_pin)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin_core::LOGIN_ATTEMPTS_MAX;
    use std::fs::OpenOptions;

    #[tokio::test(start_paused = true)]
    async fn registry_is_bounded_and_preserves_dropped_unexpired_scopes() {
        let mut registry = Registry::default();
        let owner = Arc::new(SqliteConnectOptions::new());
        let state = registry
            .acquire(
                Scope::File {
                    device: 1,
                    inode: 1,
                },
                &owner,
                None,
            )
            .unwrap();
        for _ in 0..LOGIN_ATTEMPTS_MAX {
            state.admit_login_attempt().unwrap();
        }
        drop(state);
        drop(owner);
        let owner = Arc::new(SqliteConnectOptions::new());
        for inode in 2..=LOGIN_ATTEMPTS_SCOPES_MAX as u64 {
            registry
                .acquire(Scope::File { device: 1, inode }, &owner, None)
                .unwrap();
        }
        assert!(matches!(
            registry.acquire(
                Scope::File {
                    device: 1,
                    inode: 999
                },
                &owner,
                None
            ),
            Err(AuthenticationStateError::Unavailable)
        ));
        let state = registry
            .acquire(
                Scope::File {
                    device: 1,
                    inode: 1,
                },
                &owner,
                None,
            )
            .unwrap();
        assert!(matches!(
            state.admit_login_attempt(),
            Err(AuthenticationStateError::LoginAttemptsExceeded { .. })
        ));
        drop(state);
        drop(owner);
        tokio::time::advance(std::time::Duration::from_secs(60)).await;
        let owner = Arc::new(SqliteConnectOptions::new());
        assert!(
            registry
                .acquire(
                    Scope::File {
                        device: 1,
                        inode: 999
                    },
                    &owner,
                    None
                )
                .is_ok()
        );
        assert_eq!(registry.entries.len(), 1);
    }

    #[tokio::test]
    async fn fresh_stores_and_pools_share_one_physical_database()
    -> Result<(), Box<dyn std::error::Error>> {
        use crate::admin_core::AdministratorStore;
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("administrator.sqlite3");
        let options = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true);
        let first = SqlitePool::connect_with(options.clone()).await?;
        let second = SqlitePool::connect_with(options).await?;
        let first_state = super::super::SqliteAdministratorStore::new(first.clone())
            .authentication_state()
            .await
            .unwrap();
        let second_state = super::super::SqliteAdministratorStore::new(second.clone())
            .authentication_state()
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&first_state, &second_state));
        for _ in 0..LOGIN_ATTEMPTS_MAX {
            first_state.admit_login_attempt().unwrap();
        }
        assert!(matches!(
            second_state.admit_login_attempt(),
            Err(AuthenticationStateError::LoginAttemptsExceeded { .. })
        ));
        first.close().await;
        second.close().await;
        let recreated =
            SqlitePool::connect_with(SqliteConnectOptions::new().filename(&path)).await?;
        let recreated_state = super::super::SqliteAdministratorStore::new(recreated.clone())
            .authentication_state()
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&first_state, &recreated_state));
        assert!(matches!(
            recreated_state.admit_login_attempt(),
            Err(AuthenticationStateError::LoginAttemptsExceeded { .. })
        ));
        recreated.close().await;
        Ok(())
    }

    #[tokio::test]
    async fn unlinked_database_scope_stays_pinned_and_new_database_is_independent()
    -> Result<(), Box<dyn std::error::Error>> {
        use crate::admin_core::AdministratorStore;
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("administrator.sqlite3");
        let options = || {
            SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
        };
        let first = SqlitePool::connect_with(options()).await?;
        sqlx::raw_sql(crate::admin_sqlite::ADMIN_PERSISTENT_DDL)
            .execute(&first)
            .await?;
        crate::testkit::initialize_administrator_fixture(
            &first,
            "admin",
            "correct horse battery",
            1,
        )
        .await;
        let old = super::super::SqliteAdministratorStore::new(first.clone())
            .authentication_state()
            .await?;
        let before = std::fs::metadata(&path)?;
        for _ in 0..LOGIN_ATTEMPTS_MAX {
            old.admit_login_attempt()?;
        }
        first.close().await;
        drop(first);
        std::fs::remove_file(&path)?;
        let second = SqlitePool::connect_with(options()).await?;
        sqlx::raw_sql(crate::admin_sqlite::ADMIN_PERSISTENT_DDL)
            .execute(&second)
            .await?;
        // Intentionally retain the same fixed administrator identity/time:
        // scope isolation comes from the pinned physical file, not account data.
        crate::testkit::initialize_administrator_fixture(
            &second,
            "admin",
            "correct horse battery",
            1,
        )
        .await;
        let after = std::fs::metadata(&path)?;
        assert_ne!((before.dev(), before.ino()), (after.dev(), after.ino()));
        let new = super::super::SqliteAdministratorStore::new(second.clone())
            .authentication_state()
            .await?;
        assert!(!Arc::ptr_eq(&old, &new));
        new.admit_login_attempt()?;
        assert!(matches!(
            old.admit_login_attempt(),
            Err(AuthenticationStateError::LoginAttemptsExceeded { .. })
        ));
        second.close().await;
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn expired_unowned_registry_entry_releases_file_and_memory_identity_pins() {
        use std::os::fd::AsRawFd;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("scope");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        let metadata = file.metadata().unwrap();
        let descriptor_path = format!("/proc/self/fd/{}", file.as_raw_fd());
        let scope = Scope::File {
            device: metadata.dev(),
            inode: metadata.ino(),
        };
        let owner = Arc::new(SqliteConnectOptions::new());
        let mut registry = Registry::default();
        let state = registry.acquire(scope, &owner, Some(file)).unwrap();
        state.admit_login_attempt().unwrap();
        assert!(registry.entries[&scope]._database_identity_pin.is_some());
        let memory_owner = Arc::new(SqliteConnectOptions::new());
        let weak = Arc::downgrade(&memory_owner);
        let memory_scope = Scope::Memory(Arc::as_ptr(&memory_owner) as usize);
        let memory_state = registry.acquire(memory_scope, &memory_owner, None).unwrap();
        memory_state.admit_login_attempt().unwrap();
        drop(memory_owner);
        drop(memory_state);
        drop(owner);
        drop(state);
        assert!(weak.upgrade().is_some());
        assert_eq!(registry.entries.len(), 2);
        tokio::time::advance(std::time::Duration::from_secs(60)).await;
        let next_owner = Arc::new(SqliteConnectOptions::new());
        let next_scope = Scope::Memory(Arc::as_ptr(&next_owner) as usize);
        registry.acquire(next_scope, &next_owner, None).unwrap();
        assert!(!registry.entries.contains_key(&scope));
        assert!(!registry.entries.contains_key(&memory_scope));
        assert!(weak.upgrade().is_none());
        assert_eq!(registry.entries.len(), 1);
        // Another parallel test may already reuse the descriptor number, but
        // it must no longer identify this entry's original database inode.
        match std::fs::metadata(&descriptor_path) {
            Ok(current) => assert_ne!(
                (current.dev(), current.ino()),
                (metadata.dev(), metadata.ino())
            ),
            Err(error) => assert_eq!(error.kind(), std::io::ErrorKind::NotFound),
        }
    }

    #[test]
    #[ignore = "isolated child process used by the real writer-lock regression"]
    fn sqlite_writer_probe_child() -> Result<(), Box<dyn std::error::Error>> {
        use sqlx::Connection as _;
        let path = std::env::var("XCSS_TEST_WRITER_DATABASE")?;
        let expected_busy = std::env::var("XCSS_TEST_WRITER_BUSY")? == "true";
        crate::sqlite::block_on_sqlite_connection(async {
            let mut connection = sqlx::SqliteConnection::connect_with(
                &SqliteConnectOptions::new()
                    .filename(path)
                    .create_if_missing(false)
                    .busy_timeout(std::time::Duration::from_millis(20)),
            )
            .await?;
            let result = sqlx::query("BEGIN IMMEDIATE")
                .execute(&mut connection)
                .await;
            match result {
                Ok(_) => {
                    sqlx::query("ROLLBACK").execute(&mut connection).await?;
                    assert!(
                        !expected_busy,
                        "another process acquired the SQLite writer lock while its owner still held a transaction"
                    );
                }
                Err(error) => {
                    assert!(expected_busy, "writer lock remained after commit: {error}");
                    assert!(
                        matches!(error, sqlx::Error::Database(ref error)
                        if error.code().as_deref() == Some("5")),
                        "unexpected writer probe error: {error}"
                    );
                }
            }
            connection.close().await?;
            Ok::<_, Box<dyn std::error::Error>>(())
        })
    }

    fn assert_child_writer(path: &std::path::Path, blocked: bool) {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .env_clear()
            .env("XCSS_TEST_WRITER_DATABASE", path)
            .env("XCSS_TEST_WRITER_BUSY", blocked.to_string())
            .args([
                "--exact",
                "admin_sqlite::login_attempts::tests::sqlite_writer_probe_child",
                "--ignored",
                "--nocapture",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "isolated SQLite writer probe failed: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
    }

    #[tokio::test]
    async fn repeated_scope_lookup_preserves_live_sqlite_writer_lock()
    -> Result<(), Box<dyn std::error::Error>> {
        use crate::admin_core::AdministratorStore;
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("writer.sqlite3");
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .min_connections(2)
            .max_connections(2)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(true)
                    .journal_mode(sqlx::sqlite::SqliteJournalMode::Delete),
            )
            .await?;
        sqlx::query("CREATE TABLE writer_probe(value INTEGER NOT NULL)")
            .execute(&pool)
            .await?;
        let store = super::super::SqliteAdministratorStore::new(pool.clone());
        let state = store.authentication_state().await?;
        let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
        sqlx::query("INSERT INTO writer_probe VALUES(1)")
            .execute(&mut *transaction)
            .await?;
        assert_child_writer(&path, true);
        for _ in 0..3 {
            // Each request can construct a fresh store. Reusing `store` would
            // only hit its OnceLock and never exercise the physical-scope lookup.
            let next = super::super::SqliteAdministratorStore::new(pool.clone())
                .authentication_state()
                .await?;
            assert!(Arc::ptr_eq(&state, &next));
            assert_child_writer(&path, true);
        }
        transaction.commit().await?;
        assert_child_writer(&path, false);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM writer_probe")
                .fetch_one(&pool)
                .await?,
            1
        );
        pool.close().await;
        Ok(())
    }
}
