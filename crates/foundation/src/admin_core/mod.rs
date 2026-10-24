//! Framework- and storage-independent administrator control-plane contract.

mod login_attempts;
mod management;
pub use login_attempts::{AuthenticationState, AuthenticationStateError};
pub use management::{AdministratorManagementContext, AdministratorMutation, ManagementError};

use crate::admin_auth::{
    AdministratorOriginMode, require_canonical_administrator_username,
    require_current_password_hash,
};
use std::{
    fmt,
    sync::{Arc, OnceLock},
    time::Duration,
};
use thiserror::Error;
use tokio::sync::Semaphore;

pub const POLICY_REVISION: u32 = 1;
pub const SESSION_IDLE_MICROS: u64 = 30 * 60 * 1_000_000;
pub const SESSION_ABSOLUTE_MICROS: u64 = 12 * 60 * 60 * 1_000_000;
pub const SESSIONS_PER_ADMINISTRATOR: usize = 32;
pub const SESSIONS_GLOBAL: usize = 1_024;
pub const ARGON2_CONCURRENCY: usize = 2;
pub const LOGIN_ATTEMPTS_MAX: u32 = 60;
pub const LOGIN_ATTEMPTS_WINDOW: Duration = Duration::from_secs(60);
pub const LOGIN_ATTEMPTS_SCOPES_MAX: usize = 256;
pub const LOGIN_ATTEMPTS_ERROR_CODE: &str = "auth.login_attempts_exceeded";
pub const ARGON2_ACQUIRE_TIMEOUT_MICROS: u64 = 2 * 1_000_000;
pub const SESSION_LAST_SEEN_WRITE_INTERVAL_MICROS: u64 = 60 * 1_000_000;
pub const DIGEST_BYTES: usize = 32;
pub const IDENTIFIER_MAX_BYTES: usize = 64;
pub const ADMIN_BODY_MAX_BYTES: usize = 16 * 1024;
pub const ADMIN_BODY_TIMEOUT: Duration = Duration::from_secs(10);
pub const ADMIN_BODY_READERS_GLOBAL: usize = 32;
pub const ADMINISTRATORS_REQUIRED: usize = 1;
pub const AUDIT_DETAILS_MAX_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AdministratorPolicy;

impl AdministratorPolicy {
    pub const fn revision(self) -> u32 {
        POLICY_REVISION
    }
    pub const fn session_idle_micros(self) -> u64 {
        SESSION_IDLE_MICROS
    }
    pub const fn session_absolute_micros(self) -> u64 {
        SESSION_ABSOLUTE_MICROS
    }
    pub const fn sessions_per_administrator(self) -> usize {
        SESSIONS_PER_ADMINISTRATOR
    }
    pub const fn sessions_global(self) -> usize {
        SESSIONS_GLOBAL
    }
    pub const fn argon2_concurrency(self) -> usize {
        ARGON2_CONCURRENCY
    }
    pub const fn login_attempts_max(self) -> u32 {
        LOGIN_ATTEMPTS_MAX
    }
    pub const fn login_attempts_window(self) -> Duration {
        LOGIN_ATTEMPTS_WINDOW
    }
    pub const fn login_attempts_scopes_max(self) -> usize {
        LOGIN_ATTEMPTS_SCOPES_MAX
    }
    pub const fn argon2_acquire_timeout_micros(self) -> u64 {
        ARGON2_ACQUIRE_TIMEOUT_MICROS
    }
    pub const fn last_seen_write_interval_micros(self) -> u64 {
        SESSION_LAST_SEEN_WRITE_INTERVAL_MICROS
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Identifier(String);

impl Identifier {
    pub fn new(value: impl Into<String>) -> Result<Self, Error> {
        let value = value.into();
        if value.is_empty()
            || value.len() > IDENTIFIER_MAX_BYTES
            || !value.is_ascii()
            || value
                .bytes()
                .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
        {
            return Err(Error::InvalidIdentifier);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Identifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct AdministratorRecord {
    pub administrator_id: Identifier,
    pub username: String,
    pub password_hash: String,
    pub active: bool,
    pub session_version: u64,
    pub created_at_micros: u64,
    pub updated_at_micros: u64,
    pub last_login_at_micros: Option<u64>,
}

impl fmt::Debug for AdministratorRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdministratorRecord")
            .field("administrator_id", &self.administrator_id)
            .field("username", &self.username)
            .field("active", &self.active)
            .field("password_hash", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl AdministratorRecord {
    pub fn validate(&self) -> Result<(), Error> {
        require_canonical_administrator_username(&self.username)?;
        require_current_password_hash(&self.password_hash)?;
        require_persistable_time(self.created_at_micros)?;
        require_persistable_time(self.updated_at_micros)?;
        if self.session_version == 0
            || self.updated_at_micros < self.created_at_micros
            || self
                .last_login_at_micros
                .is_some_and(|value| value > i64::MAX as u64)
        {
            return Err(Error::InvalidAdministratorRecord);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionRecord {
    pub session_id: Identifier,
    pub administrator_id: Identifier,
    pub token_hash: [u8; DIGEST_BYTES],
    pub csrf_hash: [u8; DIGEST_BYTES],
    pub administrator_session_version: u64,
    pub created_at_micros: u64,
    pub last_seen_at_micros: u64,
    pub idle_expires_at_micros: u64,
    pub absolute_expires_at_micros: u64,
    pub revoked_at_micros: Option<u64>,
}

impl SessionRecord {
    pub fn validate(&self) -> Result<(), Error> {
        for value in [
            self.created_at_micros,
            self.last_seen_at_micros,
            self.idle_expires_at_micros,
            self.absolute_expires_at_micros,
        ] {
            require_persistable_time(value)?;
        }
        if self.administrator_session_version == 0
            || self.last_seen_at_micros < self.created_at_micros
            || self.idle_expires_at_micros <= self.created_at_micros
            || self.absolute_expires_at_micros < self.idle_expires_at_micros
            || self
                .revoked_at_micros
                .is_some_and(|value| value > i64::MAX as u64)
        {
            return Err(Error::InvalidSessionRecord);
        }
        Ok(())
    }

    pub fn status(&self, administrator: &AdministratorRecord, now_micros: u64) -> SessionStatus {
        if self.revoked_at_micros.is_some() {
            SessionStatus::Revoked
        } else if !administrator.active {
            SessionStatus::AdministratorInactive
        } else if self.administrator_session_version != administrator.session_version {
            SessionStatus::AdministratorVersionChanged
        } else if now_micros >= self.absolute_expires_at_micros {
            SessionStatus::AbsoluteExpired
        } else if now_micros >= self.idle_expires_at_micros {
            SessionStatus::IdleExpired
        } else {
            SessionStatus::Active
        }
    }

    pub fn should_write_last_seen(&self, now_micros: u64) -> bool {
        now_micros.saturating_sub(self.last_seen_at_micros)
            >= SESSION_LAST_SEEN_WRITE_INTERVAL_MICROS
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionStatus {
    Active,
    Revoked,
    AdministratorInactive,
    AdministratorVersionChanged,
    IdleExpired,
    AbsoluteExpired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditOutcome {
    Success,
    Failure,
}

impl AuditOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecurityAction {
    AdministratorCreated,
    AdministratorDisabled,
    AdministratorPasswordChanged,
    AdministratorAccountUpdated,
    AdministratorSessionsRevoked,
    SessionCreated,
    SessionRevoked,
    LoginSucceeded,
}

impl SecurityAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AdministratorCreated => "administrator.created",
            Self::AdministratorDisabled => "administrator.disabled",
            Self::AdministratorAccountUpdated => "administrator.account_updated",
            Self::AdministratorPasswordChanged => "administrator.password_changed",
            Self::AdministratorSessionsRevoked => "administrator.sessions_revoked",
            Self::SessionCreated => "session.created",
            Self::SessionRevoked => "session.revoked",
            Self::LoginSucceeded => "login.succeeded",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SecurityAuditEvent {
    pub event_id: Identifier,
    pub action: SecurityAction,
    pub outcome: AuditOutcome,
    pub actor_administrator_id: Option<Identifier>,
    pub subject_digest: Option<[u8; DIGEST_BYTES]>,
    pub request_id: Option<String>,
    pub detail_json: String,
    pub occurred_at_micros: u64,
}

impl SecurityAuditEvent {
    pub fn validate(&self) -> Result<(), Error> {
        require_persistable_time(self.occurred_at_micros)?;
        if self.detail_json.len() > AUDIT_DETAILS_MAX_BYTES {
            return Err(Error::InvalidAuditEvent);
        }
        let detail: serde_json::Value =
            serde_json::from_str(&self.detail_json).map_err(|_| Error::InvalidAuditEvent)?;
        if !detail.is_object()
            || self
                .request_id
                .as_ref()
                .is_some_and(|value| crate::contracts::RequestId::new(value.clone()).is_err())
        {
            return Err(Error::InvalidAuditEvent);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoginSuccess {
    pub session: SessionRecord,
    pub session_created_event: SecurityAuditEvent,
    pub login_succeeded_event: SecurityAuditEvent,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionAndAdministrator {
    pub session: SessionRecord,
    pub administrator: AdministratorRecord,
}

#[async_trait::async_trait]
pub trait AdministratorStore: Send + Sync {
    type StoreError: std::error::Error + Send + Sync + 'static;
    /// Return the installation's shared admission state, including when a
    /// service or pool is recreated. Never derive scope from request input.
    async fn authentication_state(
        &self,
    ) -> Result<Arc<AuthenticationState>, AuthenticationStateError>;
    fn supports_account_updates(&self) -> bool;
    async fn manage_administrator(
        &self,
        context: &AdministratorManagementContext,
        mutation: AdministratorMutation,
    ) -> Result<(), ManagementError<Self::StoreError>>;
    async fn administrator_count(&self) -> Result<u64, Self::StoreError>;
    async fn administrator_by_username(
        &self,
        username: &str,
    ) -> Result<Option<AdministratorRecord>, Self::StoreError>;
    async fn session_by_token_hash(
        &self,
        token_hash: [u8; DIGEST_BYTES],
    ) -> Result<Option<SessionAndAdministrator>, Self::StoreError>;
    async fn create_administrator(
        &self,
        administrator: AdministratorRecord,
        event: SecurityAuditEvent,
    ) -> Result<(), Self::StoreError>;
    async fn change_password(
        &self,
        administrator_id: &Identifier,
        password_hash: &str,
        now_micros: u64,
        events: [SecurityAuditEvent; 2],
    ) -> Result<(), Self::StoreError>;
    async fn commit_login_success(&self, login: LoginSuccess) -> Result<(), Self::StoreError>;
    async fn touch_session(
        &self,
        session_id: &Identifier,
        expected_csrf_hash: [u8; DIGEST_BYTES],
        now_micros: u64,
        idle_expires_at_micros: u64,
    ) -> Result<bool, Self::StoreError>;
    async fn revoke_session(
        &self,
        session_id: &Identifier,
        now_micros: u64,
        event: SecurityAuditEvent,
    ) -> Result<(), Self::StoreError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoginContext {
    pub request_id: Option<String>,
    pub now_micros: u64,
}

impl LoginContext {
    pub fn validate(&self) -> Result<(), Error> {
        require_persistable_time(self.now_micros)?;
        if let Some(request_id) = &self.request_id {
            crate::contracts::RequestId::new(request_id.clone())
                .map_err(|_| Error::InvalidRequestId)?;
        }
        Ok(())
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct AuthenticatedSession {
    pub administrator: crate::contracts::AdministratorSession,
    pub session_token: String,
}

impl fmt::Debug for AuthenticatedSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthenticatedSession")
            .field("administrator", &self.administrator)
            .field("session_token", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthenticatedIdentity {
    pub administrator_id: Identifier,
    pub username: String,
    pub session_id: Identifier,
    pub csrf_hash: [u8; DIGEST_BYTES],
}

/// Complete storage-independent login/session orchestration. HTTP adapters are
/// intentionally limited to wire validation and error/cookie translation.
#[derive(Debug)]
pub struct AdministratorService<Store> {
    store: Store,
    argon2_slots: Arc<Semaphore>,
}

impl<Store> AdministratorService<Store>
where
    Store: AdministratorStore,
{
    pub fn new(store: Store) -> Self {
        Self {
            store,
            argon2_slots: {
                static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
                SLOTS
                    .get_or_init(|| Arc::new(Semaphore::new(ARGON2_CONCURRENCY)))
                    .clone()
            },
        }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    async fn hash_password(
        &self,
        password: &str,
    ) -> Result<String, ServiceError<Store::StoreError>> {
        crate::admin_auth::validate_password(password)?;
        let permit = tokio::time::timeout(
            Duration::from_micros(ARGON2_ACQUIRE_TIMEOUT_MICROS),
            self.argon2_slots.clone().acquire_owned(),
        )
        .await
        .map_err(|_| ServiceError::AuthenticationBusy)?
        .map_err(|_| ServiceError::AuthenticationBusy)?;
        let password = password.to_owned();
        tokio::task::spawn_blocking(move || {
            let result = crate::admin_auth::hash_password(&password);
            drop(permit);
            result
        })
        .await
        .map_err(|_| ServiceError::AuthenticationWorkerFailed)?
        .map_err(ServiceError::Authentication)
    }

    pub async fn login(
        &self,
        username_candidate: &str,
        password: &str,
        context: &LoginContext,
    ) -> Result<AuthenticatedSession, ServiceError<Store::StoreError>> {
        context.validate()?;
        self.store
            .authentication_state()
            .await
            .and_then(|state| state.admit_login_attempt())
            .map_err(|error| match error {
                AuthenticationStateError::Unavailable => ServiceError::AuthenticationBusy,
                AuthenticationStateError::LoginAttemptsExceeded {
                    retry_after_seconds,
                } => ServiceError::LoginAttemptsExceeded {
                    retry_after_seconds,
                },
            })?;
        let normalized =
            crate::admin_auth::normalize_administrator_username(username_candidate).ok();
        let administrator = match normalized.as_deref() {
            Some(username) => self
                .store
                .administrator_by_username(username)
                .await
                .map_err(ServiceError::Store)?,
            None => None,
        };
        let encoded = administrator
            .as_ref()
            .map_or(DUMMY_PASSWORD_HASH, |value| value.password_hash.as_str());
        let permit = tokio::time::timeout(
            Duration::from_micros(ARGON2_ACQUIRE_TIMEOUT_MICROS),
            self.argon2_slots.clone().acquire_owned(),
        )
        .await
        .map_err(|_| ServiceError::AuthenticationBusy)?
        .map_err(|_| ServiceError::AuthenticationBusy)?;
        let password = password.to_owned();
        let encoded = encoded.to_owned();
        let password_matches = tokio::task::spawn_blocking(move || {
            let matches = crate::admin_auth::verify_password(&password, &encoded);
            drop(permit);
            matches
        })
        .await
        .map_err(|_| ServiceError::AuthenticationWorkerFailed)?;
        let Some(administrator) = administrator.filter(|value| value.active && password_matches)
        else {
            return Err(ServiceError::InvalidCredentials);
        };

        let session_token = crate::admin_auth::random_token()?;
        let csrf_token = crate::admin_auth::derive_csrf_token(&session_token)?;
        let idle_expires_at_micros = context
            .now_micros
            .checked_add(SESSION_IDLE_MICROS)
            .ok_or(Error::InvalidTimestamp)?;
        let absolute_expires_at_micros = context
            .now_micros
            .checked_add(SESSION_ABSOLUTE_MICROS)
            .ok_or(Error::InvalidTimestamp)?;
        let session = SessionRecord {
            session_id: Identifier::new(crate::admin_auth::random_token()?)?,
            administrator_id: administrator.administrator_id.clone(),
            token_hash: crate::admin_auth::token_hash(&session_token),
            csrf_hash: crate::admin_auth::token_hash(&csrf_token),
            administrator_session_version: administrator.session_version,
            created_at_micros: context.now_micros,
            last_seen_at_micros: context.now_micros,
            idle_expires_at_micros,
            absolute_expires_at_micros,
            revoked_at_micros: None,
        };
        session.validate()?;
        self.store
            .commit_login_success(LoginSuccess {
                session,
                session_created_event: security_event(
                    SecurityAction::SessionCreated,
                    &administrator,
                    context,
                )?,
                login_succeeded_event: security_event(
                    SecurityAction::LoginSucceeded,
                    &administrator,
                    context,
                )?,
            })
            .await
            .map_err(ServiceError::Store)?;
        Ok(AuthenticatedSession {
            administrator: crate::contracts::AdministratorSession::new(
                administrator.administrator_id.as_str(),
                administrator.username,
                &csrf_token,
            )
            .map_err(ServiceError::Contract)?,
            session_token,
        })
    }

    pub async fn restore_session(
        &self,
        session_token: &str,
        now_micros: u64,
    ) -> Result<AuthenticatedSession, ServiceError<Store::StoreError>> {
        let identity = self.authenticate_session(session_token, now_micros).await?;
        let csrf_token = crate::admin_auth::derive_csrf_token(session_token)?;
        Ok(AuthenticatedSession {
            administrator: crate::contracts::AdministratorSession::new(
                identity.administrator_id.as_str(),
                identity.username,
                &csrf_token,
            )
            .map_err(ServiceError::Contract)?,
            session_token: session_token.to_owned(),
        })
    }

    pub async fn authenticate_session(
        &self,
        session_token: &str,
        now_micros: u64,
    ) -> Result<AuthenticatedIdentity, ServiceError<Store::StoreError>> {
        if !crate::admin_auth::is_token_shape(session_token) {
            return Err(ServiceError::InvalidSession);
        }
        require_persistable_time(now_micros)?;
        let Some(mut found) = self
            .store
            .session_by_token_hash(crate::admin_auth::token_hash(session_token))
            .await
            .map_err(ServiceError::Store)?
        else {
            return Err(ServiceError::InvalidSession);
        };
        if found.session.status(&found.administrator, now_micros) != SessionStatus::Active
            || !crate::admin_auth::token_matches_hash(
                &crate::admin_auth::derive_csrf_token(session_token)?,
                &found.session.csrf_hash,
            )
        {
            return Err(ServiceError::InvalidSession);
        }
        if found.session.should_write_last_seen(now_micros) {
            let idle_expires = now_micros
                .checked_add(SESSION_IDLE_MICROS)
                .ok_or(Error::InvalidTimestamp)?
                .min(found.session.absolute_expires_at_micros);
            let updated = self
                .store
                .touch_session(
                    &found.session.session_id,
                    found.session.csrf_hash,
                    now_micros,
                    idle_expires,
                )
                .await
                .map_err(ServiceError::Store)?;
            if !updated {
                // Recheck the current session after an activity update race instead of
                // treating a failed activity write as proof of logout.
                found = self
                    .store
                    .session_by_token_hash(crate::admin_auth::token_hash(session_token))
                    .await
                    .map_err(ServiceError::Store)?
                    .ok_or(ServiceError::InvalidSession)?;
                if found.session.status(&found.administrator, now_micros) != SessionStatus::Active
                    || !crate::admin_auth::token_matches_hash(
                        &crate::admin_auth::derive_csrf_token(session_token)?,
                        &found.session.csrf_hash,
                    )
                {
                    return Err(ServiceError::InvalidSession);
                }
            }
        }
        Ok(AuthenticatedIdentity {
            administrator_id: found.administrator.administrator_id,
            username: found.administrator.username,
            session_id: found.session.session_id,
            csrf_hash: found.session.csrf_hash,
        })
    }

    pub async fn logout(
        &self,
        session_token: &str,
        now_micros: u64,
        request_id: Option<String>,
    ) -> Result<(), ServiceError<Store::StoreError>> {
        if !crate::admin_auth::is_token_shape(session_token) {
            return Err(ServiceError::InvalidSession);
        }
        require_persistable_time(now_micros)?;
        let Some(found) = self
            .store
            .session_by_token_hash(crate::admin_auth::token_hash(session_token))
            .await
            .map_err(ServiceError::Store)?
        else {
            return Err(ServiceError::InvalidSession);
        };
        if found.session.status(&found.administrator, now_micros) != SessionStatus::Active
            || !crate::admin_auth::token_matches_hash(
                &crate::admin_auth::derive_csrf_token(session_token)?,
                &found.session.csrf_hash,
            )
        {
            return Err(ServiceError::InvalidSession);
        }
        self.store
            .revoke_session(
                &found.session.session_id,
                now_micros,
                SecurityAuditEvent {
                    event_id: Identifier::new(crate::admin_auth::random_token()?)?,
                    action: SecurityAction::SessionRevoked,
                    outcome: AuditOutcome::Success,
                    actor_administrator_id: Some(found.administrator.administrator_id),
                    subject_digest: Some(crate::admin_auth::token_hash(
                        &found.administrator.username,
                    )),
                    request_id,
                    detail_json: "{}".into(),
                    occurred_at_micros: now_micros,
                },
            )
            .await
            .map_err(ServiceError::Store)
    }

    pub async fn require_csrf(
        &self,
        session_token: &str,
        csrf_header_values: &[Vec<u8>],
        now_micros: u64,
    ) -> Result<(), ServiceError<Store::StoreError>> {
        if !crate::admin_auth::is_token_shape(session_token) {
            return Err(ServiceError::InvalidSession);
        }
        let Some(found) = self
            .store
            .session_by_token_hash(crate::admin_auth::token_hash(session_token))
            .await
            .map_err(ServiceError::Store)?
        else {
            return Err(ServiceError::InvalidSession);
        };
        if found.session.status(&found.administrator, now_micros) != SessionStatus::Active
            || !crate::admin_auth::token_matches_hash(
                &crate::admin_auth::derive_csrf_token(session_token)?,
                &found.session.csrf_hash,
            )
        {
            return Err(ServiceError::InvalidSession);
        }
        crate::admin_auth::require_csrf_token_matches_hash(
            csrf_header_values,
            &found.session.csrf_hash,
        )
        .map_err(ServiceError::Authentication)
    }
}

const DUMMY_PASSWORD_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$MDEyMzQ1Njc4OWFiY2RlZg$5ZQ3RjSgS3eazD2qv8ZO2Cm50ONQ2RplT/MMLbScSMU";

fn security_event(
    action: SecurityAction,
    administrator: &AdministratorRecord,
    context: &LoginContext,
) -> Result<SecurityAuditEvent, Error> {
    Ok(SecurityAuditEvent {
        event_id: Identifier::new(crate::admin_auth::random_token()?)?,
        action,
        outcome: AuditOutcome::Success,
        actor_administrator_id: Some(administrator.administrator_id.clone()),
        subject_digest: Some(crate::admin_auth::token_hash(&administrator.username)),
        request_id: context.request_id.clone(),
        detail_json: "{}".into(),
        occurred_at_micros: context.now_micros,
    })
}

#[derive(Debug, Error)]
pub enum ServiceError<StoreError>
where
    StoreError: std::error::Error + Send + Sync + 'static,
{
    #[error("administrator credentials are invalid")]
    InvalidCredentials,
    #[error("administrator login attempts exceeded; retry after {retry_after_seconds} seconds")]
    LoginAttemptsExceeded { retry_after_seconds: u64 },
    #[error("password verification capacity is temporarily exhausted")]
    AuthenticationBusy,
    #[error("password verification worker exited unexpectedly")]
    AuthenticationWorkerFailed,
    #[error("administrator session is invalid or expired")]
    InvalidSession,
    #[error(transparent)]
    Policy(#[from] Error),
    #[error("administrator store failed: {0}")]
    Store(StoreError),
    #[error("administrator response contract failed: {0}")]
    Contract(crate::contracts::ValidationError),
    #[error("authentication primitive failed: {0}")]
    Authentication(#[from] crate::admin_auth::Error),
}

pub fn session_cookie_name(
    product_id: &str,
    mode: AdministratorOriginMode,
) -> Result<String, Error> {
    require_product_id(product_id)?;
    let prefix = match mode {
        AdministratorOriginMode::ProductionHttps => "__Host-admin-",
        AdministratorOriginMode::LoopbackDevelopmentHttp => "admin-",
    };
    Ok(format!("{prefix}{product_id}-session"))
}

pub fn session_set_cookie(
    product_id: &str,
    mode: AdministratorOriginMode,
    token: &str,
) -> Result<String, Error> {
    if !crate::admin_auth::is_token_shape(token) {
        return Err(Error::InvalidSessionToken);
    }
    let name = session_cookie_name(product_id, mode)?;
    let secure = match mode {
        AdministratorOriginMode::ProductionHttps => "; Secure",
        AdministratorOriginMode::LoopbackDevelopmentHttp => "",
    };
    Ok(format!(
        "{name}={token}; Path=/; HttpOnly{secure}; SameSite=Strict"
    ))
}

pub fn session_clear_cookie(
    product_id: &str,
    mode: AdministratorOriginMode,
) -> Result<String, Error> {
    let name = session_cookie_name(product_id, mode)?;
    let secure = match mode {
        AdministratorOriginMode::ProductionHttps => "; Secure",
        AdministratorOriginMode::LoopbackDevelopmentHttp => "",
    };
    Ok(format!(
        "{name}=; Path=/; HttpOnly{secure}; SameSite=Strict; Max-Age=0"
    ))
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
        Err(Error::InvalidProductId)
    } else {
        Ok(())
    }
}

fn require_persistable_time(value: u64) -> Result<(), Error> {
    if value <= i64::MAX as u64 {
        Ok(())
    } else {
        Err(Error::InvalidTimestamp)
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum Error {
    #[error("identifier must be 1 to 64 visible non-whitespace ASCII bytes")]
    InvalidIdentifier,
    #[error("product ID is not canonical")]
    InvalidProductId,
    #[error("session token is not one canonical xcss token")]
    InvalidSessionToken,
    #[error("timestamp does not fit the persisted signed 64-bit representation")]
    InvalidTimestamp,
    #[error("administrator record violates current policy")]
    InvalidAdministratorRecord,
    #[error("session record violates current policy")]
    InvalidSessionRecord,
    #[error("security audit event violates current policy")]
    InvalidAuditEvent,
    #[error("request ID is invalid")]
    InvalidRequestId,
    #[error(transparent)]
    Authentication(#[from] crate::admin_auth::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_exposes_password_hashes_or_session_secrets() {
        let record = AdministratorRecord {
            administrator_id: Identifier::new("admin-1").unwrap(),
            username: "admin".into(),
            password_hash: "DO_NOT_LOG_PASSWORD_HASH".into(),
            active: true,
            session_version: 1,
            created_at_micros: 1,
            updated_at_micros: 1,
            last_login_at_micros: None,
        };
        assert!(!format!("{record:?}").contains("DO_NOT_LOG_PASSWORD_HASH"));
        let csrf = "A".repeat(43);
        let token = format!("{}E", "B".repeat(42));
        let authenticated = AuthenticatedSession {
            administrator: crate::contracts::AdministratorSession::new("admin-1", "admin", &csrf)
                .unwrap(),
            session_token: token.clone(),
        };
        let debug = format!("{authenticated:?}");
        assert!(!debug.contains(&csrf));
        assert!(!debug.contains(&token));
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn policy_and_cookie_contract_are_fixed() {
        let policy = AdministratorPolicy;
        assert_eq!(policy.session_idle_micros(), 1_800_000_000);
        assert_eq!(policy.sessions_global(), 1_024);
        let token = "A".repeat(43);
        assert_eq!(
            session_set_cookie(
                "example-product",
                AdministratorOriginMode::ProductionHttps,
                &token
            )
            .unwrap(),
            format!(
                "__Host-admin-example-product-session={token}; Path=/; HttpOnly; Secure; SameSite=Strict"
            )
        );
        assert!(
            !session_clear_cookie(
                "example-product",
                AdministratorOriginMode::LoopbackDevelopmentHttp
            )
            .unwrap()
            .contains("Secure")
        );
    }

    #[tokio::test]
    async fn independent_installations_share_cpu_budget_but_not_attempt_counts() {
        let first = AdministratorService::new(
            crate::testkit::administrator_fixture("admin", "correct horse battery").await,
        );
        let second = AdministratorService::new(
            crate::testkit::administrator_fixture("admin", "correct horse battery").await,
        );
        assert!(Arc::ptr_eq(&first.argon2_slots, &second.argon2_slots));
        let first_state = first.store().authentication_state().await.unwrap();
        let second_state = second.store().authentication_state().await.unwrap();
        assert!(!Arc::ptr_eq(&first_state, &second_state));
        for _ in 0..LOGIN_ATTEMPTS_MAX {
            first_state.admit_login_attempt().unwrap();
        }
        assert!(matches!(
            first_state.admit_login_attempt(),
            Err(AuthenticationStateError::LoginAttemptsExceeded { .. })
        ));
        assert_eq!(second_state.admit_login_attempt(), Ok(()));
    }

    #[test]
    fn audit_input_is_bounded_before_json_parsing_and_uses_canonical_request_ids() {
        let mut event = SecurityAuditEvent {
            event_id: Identifier::new("event-1").unwrap(),
            action: SecurityAction::LoginSucceeded,
            outcome: AuditOutcome::Success,
            actor_administrator_id: None,
            subject_digest: None,
            request_id: Some("valid-request".into()),
            detail_json: "{}".into(),
            occurred_at_micros: 1,
        };
        assert!(event.validate().is_ok());
        event.detail_json = format!(
            "{{\"padding\":\"{}\"}}",
            "x".repeat(AUDIT_DETAILS_MAX_BYTES)
        );
        assert!(event.validate().is_err());
        event.detail_json = "{}".into();
        event.request_id = Some("invalid\nrequest".into());
        assert!(event.validate().is_err());
    }
}
