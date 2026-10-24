//! Current-only lifecycle and metadata checks for a Xcss platform database.
//!
//! This crate opens only an existing database whose complete schema identity
//! and platform component are exactly what the running product compiled for.

use crate::schema_identity::SchemaIdentity;
use crate::sqlite::PoolOptions;
use crate::state_file::{InstanceLock, PrivateStateDirectory, SecureStateFile};
use sqlx::{Row, SqliteConnection, SqlitePool};
use thiserror::Error;

pub const PLATFORM_GENERATION: u32 = 1;
pub const PLATFORM_SCHEMA_REVISION: u32 = 1;
pub const PLATFORM_METADATA_DDL: &str = include_str!("../../schemas/platform/v1.sql");
pub const PLATFORM_METADATA_INSERT_SQL: &str = "INSERT INTO _common_platform_metadata(singleton,platform_generation,platform_schema_revision,profile,created_at_micros) VALUES(1,?,?,?,?)";
pub const PLATFORM_METADATA_READ_SQL: &str = "SELECT typeof(singleton), singleton, typeof(platform_generation), platform_generation, typeof(platform_schema_revision), platform_schema_revision, typeof(profile), profile, typeof(created_at_micros), created_at_micros FROM _common_platform_metadata ORDER BY singleton LIMIT 2";
pub const PLATFORM_METADATA_CHECK_SQL: &str = "SELECT COUNT(*)=1 AND COALESCE(MIN(typeof(singleton)='integer' AND singleton=1 AND typeof(platform_generation)='integer' AND platform_generation=? AND typeof(platform_schema_revision)='integer' AND platform_schema_revision=? AND typeof(profile)='text' AND profile=? AND typeof(created_at_micros)='integer' AND created_at_micros>=0),0) FROM _common_platform_metadata";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProductDescriptor {
    product_id: String,
    profile: String,
    database_file: String,
}

impl ProductDescriptor {
    pub fn new(
        product_id: impl Into<String>,
        profile: impl Into<String>,
        database_file: impl Into<String>,
    ) -> Result<Self, Error> {
        let descriptor = Self {
            product_id: product_id.into(),
            profile: profile.into(),
            database_file: database_file.into(),
        };
        require_product_id(&descriptor.product_id)?;
        if descriptor.profile != "server-control-plane" {
            return Err(Error::UnsupportedProfile {
                actual: descriptor.profile,
            });
        }
        if descriptor.database_file.is_empty() {
            return Err(Error::InvalidDatabaseFile);
        }
        Ok(descriptor)
    }

    pub fn product_id(&self) -> &str {
        &self.product_id
    }

    pub fn profile(&self) -> &str {
        &self.profile
    }

    pub fn database_file(&self) -> &str {
        &self.database_file
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformMetadata {
    pub platform_generation: u32,
    pub platform_schema_revision: u32,
    pub profile: String,
    pub created_at_micros: u64,
}

#[derive(Debug)]
pub struct PlatformDatabase {
    pool: SqlitePool,
    metadata: PlatformMetadata,
    descriptor: ProductDescriptor,
    _secure_file: SecureStateFile,
    _instance_lock: InstanceLock,
}

impl PlatformDatabase {
    pub async fn open(
        state_directory: PrivateStateDirectory,
        product_descriptor: ProductDescriptor,
        expected_schema: SchemaIdentity,
    ) -> Result<Self, Error> {
        Self::open_with_options(
            state_directory,
            product_descriptor,
            expected_schema,
            PoolOptions::default(),
        )
        .await
    }

    pub async fn open_with_options(
        state_directory: PrivateStateDirectory,
        product_descriptor: ProductDescriptor,
        expected_schema: SchemaIdentity,
        pool_options: PoolOptions,
    ) -> Result<Self, Error> {
        if expected_schema.application != product_descriptor.product_id {
            return Err(Error::ProductIdentityMismatch {
                descriptor: product_descriptor.product_id.clone(),
                schema: expected_schema.application,
            });
        }
        state_directory.verify_identity()?;
        let instance_lock = state_directory.try_instance_lock()?;
        let secure_file = state_directory.open_existing(&product_descriptor.database_file)?;
        let pool = crate::sqlite::open_existing(secure_file.path(), pool_options).await?;
        secure_file.verify_identity()?;
        crate::sqlite::require_pool_current_schema(&pool, &expected_schema).await?;
        let metadata =
            require_current_platform_metadata(&pool, product_descriptor.profile()).await?;
        secure_file.verify_identity()?;
        Ok(Self {
            pool,
            metadata,
            descriptor: product_descriptor,
            _secure_file: secure_file,
            _instance_lock: instance_lock,
        })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub fn metadata(&self) -> &PlatformMetadata {
        &self.metadata
    }

    pub fn descriptor(&self) -> &ProductDescriptor {
        &self.descriptor
    }

    pub async fn close(self) {
        self.pool.close().await;
    }
}

pub async fn read_platform_metadata(pool: &SqlitePool) -> Result<PlatformMetadata, Error> {
    validate_platform_table(pool).await?;
    let rows = sqlx::query(PLATFORM_METADATA_READ_SQL)
        .fetch_all(pool)
        .await?;
    if rows.len() != 1 {
        return Err(Error::PlatformMetadataRowCount { actual: rows.len() });
    }
    let row = &rows[0];
    for (field, index, expected) in [
        ("singleton", 0, "integer"),
        ("platform_generation", 2, "integer"),
        ("platform_schema_revision", 4, "integer"),
        ("profile", 6, "text"),
        ("created_at_micros", 8, "integer"),
    ] {
        let actual: String = row.try_get(index)?;
        if actual != expected {
            return Err(Error::PlatformMetadataStorageClass {
                field,
                expected,
                actual,
            });
        }
    }
    let singleton: i64 = row.try_get(1)?;
    if singleton != 1 {
        return Err(Error::InvalidPlatformSingleton { actual: singleton });
    }
    Ok(PlatformMetadata {
        platform_generation: positive_u32("platform_generation", row.try_get(3)?)?,
        platform_schema_revision: positive_u32("platform_schema_revision", row.try_get(5)?)?,
        profile: row.try_get(7)?,
        created_at_micros: nonnegative_u64("created_at_micros", row.try_get(9)?)?,
    })
}

/// Inserts the single current platform metadata row while a product creates
/// its current schema. Product rows remain outside this crate.
pub async fn initialize_current_platform_metadata(
    connection: &mut SqliteConnection,
    profile: &str,
    created_at_micros: u64,
) -> Result<PlatformMetadata, Error> {
    require_supported_profile(profile)?;
    let created_at_micros = i64::try_from(created_at_micros)
        .map_err(|_| Error::PlatformTimestampOutOfRange { created_at_micros })?;
    sqlx::query(PLATFORM_METADATA_INSERT_SQL)
        .bind(i64::from(PLATFORM_GENERATION))
        .bind(i64::from(PLATFORM_SCHEMA_REVISION))
        .bind(profile)
        .bind(created_at_micros)
        .execute(connection)
        .await?;
    Ok(PlatformMetadata {
        platform_generation: PLATFORM_GENERATION,
        platform_schema_revision: PLATFORM_SCHEMA_REVISION,
        profile: profile.to_owned(),
        created_at_micros: u64::try_from(created_at_micros)
            .expect("validated platform timestamp is non-negative"),
    })
}

/// Reads and verifies the exact current platform metadata contract expected by
/// a product profile.
pub async fn require_current_platform_metadata(
    pool: &SqlitePool,
    expected_profile: &str,
) -> Result<PlatformMetadata, Error> {
    require_supported_profile(expected_profile)?;
    let metadata = read_platform_metadata(pool).await?;
    require_current_platform(&metadata, expected_profile)?;
    Ok(metadata)
}

async fn validate_platform_table(pool: &SqlitePool) -> Result<(), Error> {
    let ddl: Option<String> = sqlx::query_scalar(
        "SELECT sql FROM sqlite_schema WHERE type='table' AND name='_common_platform_metadata'",
    )
    .fetch_optional(pool)
    .await?;
    let ddl = ddl.ok_or(Error::PlatformMetadataTableMissing)?;
    if normalize_sql(&ddl) != normalize_sql(PLATFORM_METADATA_DDL) {
        return Err(Error::PlatformMetadataDdlMismatch);
    }
    Ok(())
}

fn normalize_sql(sql: &str) -> String {
    sql.chars()
        .filter(|character| !character.is_ascii_whitespace() && *character != ';')
        .flat_map(char::to_lowercase)
        .collect()
}

fn require_current_platform(
    metadata: &PlatformMetadata,
    expected_profile: &str,
) -> Result<(), Error> {
    if metadata.platform_generation != PLATFORM_GENERATION {
        return Err(Error::PlatformGenerationMismatch {
            expected: PLATFORM_GENERATION,
            actual: metadata.platform_generation,
        });
    }
    if metadata.platform_schema_revision != PLATFORM_SCHEMA_REVISION {
        return Err(Error::PlatformSchemaRevisionMismatch {
            expected: PLATFORM_SCHEMA_REVISION,
            actual: metadata.platform_schema_revision,
        });
    }
    if metadata.profile != expected_profile {
        return Err(Error::PlatformProfileMismatch {
            expected: expected_profile.to_owned(),
            actual: metadata.profile.clone(),
        });
    }
    Ok(())
}

fn require_supported_profile(profile: &str) -> Result<(), Error> {
    if profile != "server-control-plane" {
        return Err(Error::UnsupportedProfile {
            actual: profile.to_owned(),
        });
    }
    Ok(())
}

fn require_product_id(value: &str) -> Result<(), Error> {
    let bytes = value.as_bytes();
    if bytes.is_empty()
        || bytes.len() > 63
        || !bytes[0].is_ascii_lowercase()
        || !bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
    {
        return Err(Error::InvalidProductId);
    }
    Ok(())
}

fn positive_u32(field: &'static str, value: i64) -> Result<u32, Error> {
    let value = u32::try_from(value).map_err(|_| Error::InvalidPlatformInteger { field, value })?;
    if value == 0 {
        Err(Error::InvalidPlatformInteger {
            field,
            value: i64::from(value),
        })
    } else {
        Ok(value)
    }
}

fn nonnegative_u64(field: &'static str, value: i64) -> Result<u64, Error> {
    u64::try_from(value).map_err(|_| Error::InvalidPlatformInteger { field, value })
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("product ID is not canonical")]
    InvalidProductId,
    #[error("platform database requires profile server-control-plane, found {actual}")]
    UnsupportedProfile { actual: String },
    #[error("database file name is empty")]
    InvalidDatabaseFile,
    #[error("descriptor product {descriptor} differs from schema application {schema}")]
    ProductIdentityMismatch { descriptor: String, schema: String },
    #[error(transparent)]
    StateFile(#[from] crate::state_file::Error),
    #[error(transparent)]
    Sqlite(#[from] crate::sqlite::Error),
    #[error("platform metadata table is missing")]
    PlatformMetadataTableMissing,
    #[error("platform metadata DDL differs from the current xcss DDL")]
    PlatformMetadataDdlMismatch,
    #[error("platform metadata must contain exactly one row, found {actual}")]
    PlatformMetadataRowCount { actual: usize },
    #[error("platform metadata singleton must be 1, found {actual}")]
    InvalidPlatformSingleton { actual: i64 },
    #[error("platform metadata {field} must use storage class {expected}, found {actual}")]
    PlatformMetadataStorageClass {
        field: &'static str,
        expected: &'static str,
        actual: String,
    },
    #[error("platform metadata {field} has invalid integer {value}")]
    InvalidPlatformInteger { field: &'static str, value: i64 },
    #[error("platform metadata timestamp {created_at_micros} exceeds SQLite INTEGER")]
    PlatformTimestampOutOfRange { created_at_micros: u64 },
    #[error("platform generation must be {expected}, found {actual}")]
    PlatformGenerationMismatch { expected: u32, actual: u32 },
    #[error("platform schema revision must be {expected}, found {actual}")]
    PlatformSchemaRevisionMismatch { expected: u32, actual: u32 },
    #[error("platform profile must be {expected}, found {actual}")]
    PlatformProfileMismatch { expected: String, actual: String },
    #[error("SQLite platform query failed: {0}")]
    Sqlx(#[from] sqlx::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt};

    async fn current_database(
        profile: &str,
    ) -> Result<
        (tempfile::TempDir, PrivateStateDirectory, SchemaIdentity),
        Box<dyn std::error::Error>,
    > {
        let parent = tempfile::tempdir()?;
        fs::set_permissions(parent.path(), fs::Permissions::from_mode(0o700))?;
        let state = PrivateStateDirectory::create(parent.path().join("state"))?;
        state.create_file("state.sqlite3")?;
        let pool =
            crate::sqlite::open_existing(state.path().join("state.sqlite3"), PoolOptions::new(1))
                .await?;
        sqlx::raw_sql(crate::sqlite::PRODUCT_METADATA_DDL)
            .execute(&pool)
            .await?;
        sqlx::raw_sql(PLATFORM_METADATA_DDL).execute(&pool).await?;
        sqlx::query(
            "INSERT INTO _common_platform_metadata(\
               singleton, platform_generation, platform_schema_revision, profile, created_at_micros\
             ) VALUES(1, 1, 1, ?, 7)",
        )
        .bind(profile)
        .execute(&pool)
        .await?;
        let fingerprint = crate::sqlite::schema_fingerprint(&pool).await?;
        let identity = SchemaIdentity::new("example-product", "0.5.0", 1, fingerprint)?;
        sqlx::query(
            "INSERT INTO product_metadata(\
               singleton, application, application_version, schema_revision, schema_sha256\
             ) VALUES(1, ?, ?, ?, ?)",
        )
        .bind(&identity.application)
        .bind(&identity.application_version)
        .bind(i64::try_from(identity.schema_revision)?)
        .bind(&identity.schema_sha256)
        .execute(&pool)
        .await?;
        pool.close().await;
        Ok((parent, state, identity))
    }

    #[tokio::test]
    async fn opens_only_the_exact_current_platform_database()
    -> Result<(), Box<dyn std::error::Error>> {
        let (_parent, state, identity) = current_database("server-control-plane").await?;
        let descriptor =
            ProductDescriptor::new("example-product", "server-control-plane", "state.sqlite3")?;
        let database = PlatformDatabase::open(state.clone(), descriptor, identity).await?;
        assert_eq!(database.metadata().created_at_micros, 7);
        assert!(matches!(
            PlatformDatabase::open(
                state,
                ProductDescriptor::new(
                    "example-product",
                    "server-control-plane",
                    "state.sqlite3",
                )?,
                SchemaIdentity::new("example-product", "9.9.9", 1, "0".repeat(64))?,
            )
            .await,
            Err(Error::StateFile(crate::state_file::Error::LockUnavailable { .. }))
        ));
        database.close().await;
        Ok(())
    }

    #[tokio::test]
    async fn rejects_wrong_platform_profile() -> Result<(), Box<dyn std::error::Error>> {
        let (_parent, state, identity) = current_database("server-filesystem").await?;
        let error = PlatformDatabase::open(
            state,
            ProductDescriptor::new("example-product", "server-control-plane", "state.sqlite3")?,
            identity,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, Error::PlatformProfileMismatch { .. }));
        Ok(())
    }

    #[tokio::test]
    async fn initializes_and_validates_current_platform_metadata()
    -> Result<(), Box<dyn std::error::Error>> {
        let pool = SqlitePool::connect("sqlite::memory:").await?;
        sqlx::raw_sql(PLATFORM_METADATA_DDL).execute(&pool).await?;
        let mut connection = pool.acquire().await?;
        let initialized =
            initialize_current_platform_metadata(&mut connection, "server-control-plane", 42)
                .await?;
        assert_eq!(initialized.created_at_micros, 42);
        drop(connection);
        assert_eq!(
            require_current_platform_metadata(&pool, "server-control-plane").await?,
            initialized
        );
        let mut connection = pool.acquire().await?;
        assert!(matches!(
            initialize_current_platform_metadata(&mut connection, "server-control-plane", 43,)
                .await,
            Err(Error::Sqlx(_))
        ));
        assert!(matches!(
            require_current_platform_metadata(&pool, "server-filesystem").await,
            Err(Error::UnsupportedProfile { .. })
        ));
        Ok(())
    }
}
