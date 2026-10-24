//! Deterministic transition rules. Storage and product execution are adapters.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sqlx::{Acquire, Row, Sqlite, SqlitePool, Transaction};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationState {
    Pending,
    Running,
    Succeeded,
    Failed,
    Unknown,
    DeadLetter,
    Resolved,
}

/// Explicit human decisions; none of these replay an ambiguous remote effect.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    ConfirmedSucceeded,
    ConfirmedFailed,
    UnableToConfirm,
}

impl Resolution {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ConfirmedSucceeded => "confirmed_succeeded",
            Self::ConfirmedFailed => "confirmed_failed",
            Self::UnableToConfirm => "unable_to_confirm",
        }
    }
}

impl OperationState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
            Self::DeadLetter => "dead_letter",
            Self::Resolved => "resolved",
        }
    }

    pub fn parse(value: &str) -> Result<Self, Error> {
        match value {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "unknown" => Ok(Self::Unknown),
            "dead_letter" => Ok(Self::DeadLetter),
            "resolved" => Ok(Self::Resolved),
            _ => Err(Error::InvalidStoredState),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Operation {
    pub operation_id: String,
    pub namespace: String,
    pub target_key: String,
    pub idempotency_digest: [u8; 32],
    pub request_fingerprint: [u8; 32],
    pub state: OperationState,
    pub attempt: u32,
    pub max_attempts: u32,
    pub not_before_micros: i64,
    pub lease_owner: Option<String>,
    pub lease_expiry_micros: Option<i64>,
    pub error_code: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Transition {
    Claim {
        owner: String,
        lease_expiry_micros: i64,
    },
    Succeed,
    Fail {
        code: String,
        retryable: bool,
        retry_not_before_micros: i64,
    },
    MarkIndeterminate {
        code: String,
    },
    DeadLetter {
        code: String,
    },
    Resolve {
        code: String,
    },
}

pub fn transition(operation: &mut Operation, event: Transition) -> Result<(), Error> {
    match (&operation.state, event) {
        (
            OperationState::Pending,
            Transition::Claim {
                owner,
                lease_expiry_micros,
            },
        ) if lease_expiry_micros > operation.not_before_micros => {
            operation.attempt = operation
                .attempt
                .checked_add(1)
                .ok_or(Error::AttemptOverflow)?;
            operation.state = OperationState::Running;
            operation.lease_owner = Some(owner);
            operation.lease_expiry_micros = Some(lease_expiry_micros);
        }
        (OperationState::Running, Transition::Succeed) => {
            operation.state = OperationState::Succeeded;
            operation.error_code = None;
            clear_lease(operation);
        }
        (
            OperationState::Running,
            Transition::Fail {
                code,
                retryable: true,
                retry_not_before_micros,
            },
        ) => {
            operation.error_code = Some(code);
            clear_lease(operation);
            if operation.attempt >= operation.max_attempts {
                operation.state = OperationState::DeadLetter;
            } else {
                operation.state = OperationState::Pending;
                operation.not_before_micros = retry_not_before_micros;
            }
        }
        (
            OperationState::Running,
            Transition::Fail {
                code,
                retryable: false,
                ..
            },
        ) => {
            operation.state = OperationState::Failed;
            operation.error_code = Some(code);
            clear_lease(operation);
        }
        (OperationState::Running, Transition::MarkIndeterminate { code }) => {
            operation.state = OperationState::Unknown;
            operation.error_code = Some(code);
            clear_lease(operation);
        }
        (OperationState::Running, Transition::DeadLetter { code }) => {
            operation.state = OperationState::DeadLetter;
            operation.error_code = Some(code);
            clear_lease(operation);
        }
        (OperationState::Unknown, Transition::DeadLetter { code }) => {
            operation.state = OperationState::DeadLetter;
            operation.error_code = Some(code);
        }
        (
            OperationState::Unknown | OperationState::Failed | OperationState::DeadLetter,
            Transition::Resolve { code },
        ) => {
            operation.state = OperationState::Resolved;
            operation.error_code = Some(code);
            clear_lease(operation);
        }
        (state, event) => {
            return Err(Error::InvalidTransition {
                state: *state,
                event: format!("{event:?}"),
            });
        }
    }
    Ok(())
}
fn clear_lease(operation: &mut Operation) {
    operation.lease_owner = None;
    operation.lease_expiry_micros = None;
}

pub fn validate_idempotency(
    existing: &Operation,
    digest: &[u8; 32],
    fingerprint: &[u8; 32],
) -> Result<(), Error> {
    if &existing.idempotency_digest != digest {
        return Err(Error::IdempotencyKeyMismatch);
    }
    if &existing.request_fingerprint != fingerprint {
        return Err(Error::IdempotencyConflict);
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationContext {
    pub operation_id: String,
    pub namespace: String,
    pub target_key: String,
    pub attempt: u32,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OperationOutcome<T> {
    Succeeded(T),
    DefinitiveFailure { code: String, retryable: bool },
    Indeterminate { code: String },
}

#[async_trait]
pub trait OperationExecutor: Send + Sync {
    type Request: Send;
    type Result: Send;
    async fn execute(
        &self,
        context: OperationContext,
        request: Self::Request,
    ) -> OperationOutcome<Self::Result>;
}

pub const PLATFORM_OPERATIONS_DDL: &str = include_str!("../../schemas/durable-operations/v1.sql");

pub const MAX_IDENTIFIER_BYTES: usize = 256;
pub const MAX_PAYLOAD_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewOperation {
    pub operation_id: String,
    pub namespace: String,
    pub target_key: String,
    pub action: String,
    pub idempotency_digest: [u8; 32],
    pub request_fingerprint: [u8; 32],
    pub request_payload: Vec<u8>,
    pub max_attempts: u32,
    pub not_before_micros: i64,
    pub created_at_micros: i64,
}

impl NewOperation {
    pub fn validate(&self) -> Result<(), Error> {
        for value in [
            &self.operation_id,
            &self.namespace,
            &self.target_key,
            &self.action,
        ] {
            if value.is_empty()
                || value.len() > MAX_IDENTIFIER_BYTES
                || value.bytes().any(|byte| byte.is_ascii_control())
            {
                return Err(Error::InvalidRecord);
            }
        }
        if self.request_payload.len() > MAX_PAYLOAD_BYTES
            || self.max_attempts == 0
            || self.created_at_micros < 0
            || self.not_before_micros < self.created_at_micros
        {
            return Err(Error::InvalidRecord);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredOperation {
    pub operation: Operation,
    pub action: String,
    pub request_payload: Vec<u8>,
    pub result_payload: Option<Vec<u8>>,
    pub resolution_code: Option<String>,
    pub created_at_micros: i64,
    pub updated_at_micros: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EnqueueOutcome {
    Created(StoredOperation),
    Existing(StoredOperation),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditEvent {
    pub event_id: String,
    pub operation_id: String,
    pub from_state: String,
    pub to_state: String,
    pub payload_json: String,
    pub created_at_micros: i64,
}

fn matches_claim(current: &Operation, claim: &Operation) -> bool {
    current.operation_id == claim.operation_id
        && current.state == OperationState::Running
        && claim.state == OperationState::Running
        && current.attempt == claim.attempt
        && current.lease_owner == claim.lease_owner
        && current.lease_expiry_micros == claim.lease_expiry_micros
}

#[derive(Clone)]
pub struct SqliteOperationStore {
    pool: SqlitePool,
}

impl SqliteOperationStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Unmaterialized audit records, without reading their product payloads.
    pub async fn pending_audit_count(&self) -> Result<u64, Error> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM _common_operation_audit_outbox WHERE delivered_at_micros IS NULL",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(storage)?;
        u64::try_from(count).map_err(|_| Error::InvalidRecord)
    }

    /// Pending execution, in-flight work and indeterminate work blocking a target.
    pub async fn active_operation_count(&self) -> Result<u64, Error> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM _common_operations WHERE state IN ('pending','running','unknown')",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(storage)?;
        u64::try_from(count).map_err(|_| Error::InvalidRecord)
    }

    pub async fn enqueue(&self, value: NewOperation) -> Result<EnqueueOutcome, Error> {
        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage)?;
        let outcome = Self::enqueue_in(&mut transaction, value).await?;
        transaction.commit().await.map_err(storage)?;
        Ok(outcome)
    }

    /// Enqueues an operation in a caller-owned transaction so product state and
    /// the durable intent can be committed atomically.
    pub async fn enqueue_in(
        transaction: &mut Transaction<'_, Sqlite>,
        value: NewOperation,
    ) -> Result<EnqueueOutcome, Error> {
        let mut savepoint = transaction.begin().await.map_err(storage)?;
        match Self::enqueue_checked_in(&mut savepoint, value).await {
            Ok(outcome) => {
                savepoint.commit().await.map_err(storage)?;
                Ok(outcome)
            }
            Err(error) => {
                savepoint.rollback().await.map_err(storage)?;
                Err(error)
            }
        }
    }

    async fn enqueue_checked_in(
        transaction: &mut Transaction<'_, Sqlite>,
        value: NewOperation,
    ) -> Result<EnqueueOutcome, Error> {
        value.validate()?;
        if let Some(existing) =
            get_by_idempotency_in(transaction, &value.namespace, &value.idempotency_digest).await?
        {
            validate_idempotency(
                &existing.operation,
                &value.idempotency_digest,
                &value.request_fingerprint,
            )?;
            return Ok(EnqueueOutcome::Existing(existing));
        }
        let inserted = sqlx::query(
            "INSERT INTO _common_operations(\
             operation_id,namespace,target_key,action,idempotency_digest,request_fingerprint,\
             request_payload,state,attempt,max_attempts,not_before_micros,created_at_micros,updated_at_micros\
             ) VALUES(?,?,?,?,?,?,?,'pending',0,?,?,?,?)",
        )
        .bind(&value.operation_id)
        .bind(&value.namespace)
        .bind(&value.target_key)
        .bind(&value.action)
        .bind(value.idempotency_digest.as_slice())
        .bind(value.request_fingerprint.as_slice())
        .bind(&value.request_payload)
        .bind(i64::from(value.max_attempts))
        .bind(value.not_before_micros)
        .bind(value.created_at_micros)
        .bind(value.created_at_micros)
        .execute(&mut **transaction)
        .await;
        match inserted {
            Ok(_) => {
                write_audit_raw(
                    transaction,
                    &value.operation_id,
                    "none",
                    OperationState::Pending.as_str(),
                    value.created_at_micros,
                )
                .await?;
                get_by_id_in(transaction, &value.operation_id)
                    .await?
                    .map(EnqueueOutcome::Created)
                    .ok_or(Error::ConcurrentModification)
            }
            Err(error) if is_constraint(&error) => {
                let existing =
                    get_by_idempotency_in(transaction, &value.namespace, &value.idempotency_digest)
                        .await?
                        .ok_or_else(|| Error::Storage(error.to_string()))?;
                validate_idempotency(
                    &existing.operation,
                    &value.idempotency_digest,
                    &value.request_fingerprint,
                )?;
                Ok(EnqueueOutcome::Existing(existing))
            }
            Err(error) => Err(Error::Storage(error.to_string())),
        }
    }

    pub async fn get(&self, operation_id: &str) -> Result<Option<StoredOperation>, Error> {
        let row = sqlx::query(SELECT_OPERATION_BY_ID)
            .bind(operation_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(storage)?;
        row.map(decode_operation).transpose()
    }

    pub async fn get_by_idempotency(
        &self,
        namespace: &str,
        digest: &[u8; 32],
    ) -> Result<Option<StoredOperation>, Error> {
        let row = sqlx::query(SELECT_OPERATION_BY_IDEMPOTENCY)
            .bind(namespace)
            .bind(digest.as_slice())
            .fetch_optional(&self.pool)
            .await
            .map_err(storage)?;
        row.map(decode_operation).transpose()
    }

    pub async fn latest_for_target(
        &self,
        namespace: &str,
        target_key: &str,
    ) -> Result<Option<StoredOperation>, Error> {
        if namespace.is_empty() || target_key.is_empty() {
            return Err(Error::InvalidRecord);
        }
        let row = sqlx::query(SELECT_LATEST_OPERATION_FOR_TARGET)
            .bind(namespace)
            .bind(target_key)
            .fetch_optional(&self.pool)
            .await
            .map_err(storage)?;
        row.map(decode_operation).transpose()
    }

    pub async fn claim_next(
        &self,
        namespace: &str,
        owner: &str,
        now_micros: i64,
        lease_expiry_micros: i64,
    ) -> Result<Option<StoredOperation>, Error> {
        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage)?;
        let claimed = Self::claim_next_in(
            &mut transaction,
            namespace,
            owner,
            now_micros,
            lease_expiry_micros,
        )
        .await?;
        transaction.commit().await.map_err(storage)?;
        Ok(claimed)
    }

    /// Joins product-specific admission/leader fencing and claim in one transaction.
    pub async fn claim_next_in(
        transaction: &mut Transaction<'_, Sqlite>,
        namespace: &str,
        owner: &str,
        now_micros: i64,
        lease_expiry_micros: i64,
    ) -> Result<Option<StoredOperation>, Error> {
        if namespace.is_empty()
            || namespace.len() > MAX_IDENTIFIER_BYTES
            || now_micros < 0
            || owner.is_empty()
            || owner.len() > MAX_IDENTIFIER_BYTES
            || lease_expiry_micros <= now_micros
        {
            return Err(Error::InvalidRecord);
        }
        let mut savepoint = transaction.begin().await.map_err(storage)?;
        let result = async {
            let candidate: Option<String> = sqlx::query_scalar(
                "SELECT o.operation_id FROM _common_operations o \
                 WHERE o.namespace=? AND o.state='pending' AND o.not_before_micros<=? \
                   AND NOT EXISTS (SELECT 1 FROM _common_operations active \
                     WHERE active.namespace=o.namespace AND active.target_key=o.target_key \
                       AND active.state IN ('running','unknown')) \
                 ORDER BY o.not_before_micros,o.created_at_micros,o.operation_id LIMIT 1",
            )
            .bind(namespace)
            .bind(now_micros)
            .fetch_optional(&mut *savepoint)
            .await
            .map_err(storage)?;
            let Some(operation_id) = candidate else {
                return Ok(None);
            };
            let row = sqlx::query(SELECT_OPERATION_BY_ID)
                .bind(&operation_id)
                .fetch_optional(&mut *savepoint)
                .await
                .map_err(storage)?;
            let Some(mut current) = row.map(decode_operation).transpose()? else {
                return Err(Error::ConcurrentModification);
            };
            let before = current.operation.state;
            transition(
                &mut current.operation,
                Transition::Claim {
                    owner: owner.to_owned(),
                    lease_expiry_micros,
                },
            )?;
            let updated = sqlx::query(
                "UPDATE _common_operations SET state='running',attempt=?,lease_owner=?,\
                 lease_expiry_micros=?,updated_at_micros=? \
                 WHERE operation_id=? AND state='pending' AND attempt=? \
                   AND NOT EXISTS (SELECT 1 FROM _common_operations active \
                     WHERE active.namespace=_common_operations.namespace \
                       AND active.target_key=_common_operations.target_key \
                       AND active.state IN ('running','unknown'))",
            )
            .bind(i64::from(current.operation.attempt))
            .bind(owner)
            .bind(lease_expiry_micros)
            .bind(now_micros)
            .bind(&operation_id)
            .bind(i64::from(current.operation.attempt - 1))
            .execute(&mut *savepoint)
            .await;
            match updated {
                Ok(done) if done.rows_affected() == 1 => {
                    write_audit(
                        &mut savepoint,
                        &operation_id,
                        before,
                        OperationState::Running,
                        now_micros,
                    )
                    .await?;
                    get_by_id_in(&mut savepoint, &operation_id).await
                }
                Ok(_) => Err(Error::ConcurrentModification),
                Err(error) if is_constraint(&error) => Err(Error::ConcurrentModification),
                Err(error) => Err(storage(error)),
            }
        }
        .await;
        match result {
            Ok(claimed) => {
                savepoint.commit().await.map_err(storage)?;
                Ok(claimed)
            }
            Err(error) => {
                savepoint.rollback().await.map_err(storage)?;
                Err(error)
            }
        }
    }

    async fn apply_transition(
        &self,
        operation_id: &str,
        event: Transition,
        result_payload: Option<&[u8]>,
        now_micros: i64,
    ) -> Result<StoredOperation, Error> {
        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage)?;
        let updated = Self::apply_transition_checked_in(
            &mut transaction,
            operation_id,
            event,
            result_payload,
            now_micros,
            None,
        )
        .await?;
        transaction.commit().await.map_err(storage)?;
        Ok(updated)
    }

    /// Applies a transition only while the caller still owns an unexpired
    /// running lease. This fences late results from superseded workers.
    pub async fn apply_transition_owned(
        &self,
        claim: &Operation,
        event: Transition,
        result_payload: Option<&[u8]>,
        now_micros: i64,
    ) -> Result<StoredOperation, Error> {
        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage)?;
        let updated = Self::apply_transition_owned_in(
            &mut transaction,
            claim,
            event,
            result_payload,
            now_micros,
        )
        .await?;
        transaction.commit().await.map_err(storage)?;
        Ok(updated)
    }

    /// Atomically joins product writes, a fenced completion and its audit intent.
    /// A failed transition rolls back its own savepoint; callers must roll back
    /// the enclosing transaction if their product writes must not be published.
    pub async fn apply_transition_owned_in(
        transaction: &mut Transaction<'_, Sqlite>,
        claim: &Operation,
        event: Transition,
        result_payload: Option<&[u8]>,
        now_micros: i64,
    ) -> Result<StoredOperation, Error> {
        if claim.state != OperationState::Running
            || claim.lease_expiry_micros.is_none()
            || claim
                .lease_owner
                .as_ref()
                .is_none_or(|owner| owner.is_empty() || owner.len() > MAX_IDENTIFIER_BYTES)
        {
            return Err(Error::InvalidRecord);
        }
        let mut savepoint = transaction.begin().await.map_err(storage)?;
        match Self::apply_transition_checked_in(
            &mut savepoint,
            &claim.operation_id,
            event,
            result_payload,
            now_micros,
            Some(claim),
        )
        .await
        {
            Ok(updated) => {
                savepoint.commit().await.map_err(storage)?;
                Ok(updated)
            }
            Err(error) => {
                savepoint.rollback().await.map_err(storage)?;
                Err(error)
            }
        }
    }

    /// Records an ambiguous result for precisely the captured claim, including
    /// after its lease expired. This never publishes success or makes work
    /// eligible for replay, and cannot affect a later claim/attempt.
    pub async fn abandon_claim(
        &self,
        claim: &Operation,
        code: &str,
        now_micros: i64,
    ) -> Result<StoredOperation, Error> {
        if claim.state != OperationState::Running
            || claim.lease_owner.is_none()
            || claim.lease_expiry_micros.is_none()
        {
            return Err(Error::InvalidRecord);
        }
        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage)?;
        let current = get_by_id_in(&mut transaction, &claim.operation_id)
            .await?
            .ok_or(Error::NotFound)?;
        if !matches_claim(&current.operation, claim) {
            return Err(Error::ConcurrentModification);
        }
        let updated = Self::apply_transition_checked_in(
            &mut transaction,
            &claim.operation_id,
            Transition::MarkIndeterminate {
                code: code.to_owned(),
            },
            None,
            now_micros,
            None,
        )
        .await?;
        transaction.commit().await.map_err(storage)?;
        Ok(updated)
    }

    /// Human resolution and product actor audit can share the same commit.
    pub async fn resolve_in(
        transaction: &mut Transaction<'_, Sqlite>,
        operation_id: &str,
        resolution: Resolution,
        now_micros: i64,
    ) -> Result<StoredOperation, Error> {
        let mut savepoint = transaction.begin().await.map_err(storage)?;
        let current = get_by_id_in(&mut savepoint, operation_id)
            .await?
            .ok_or(Error::NotFound)?;
        let event = if resolution == Resolution::UnableToConfirm {
            if current.operation.state != OperationState::Unknown {
                return Err(Error::InvalidTransition {
                    state: current.operation.state,
                    event: "unable_to_confirm".into(),
                });
            }
            Transition::DeadLetter {
                code: resolution.as_str().into(),
            }
        } else {
            Transition::Resolve {
                code: resolution.as_str().into(),
            }
        };
        match Self::apply_transition_checked_in(
            &mut savepoint,
            operation_id,
            event,
            None,
            now_micros,
            None,
        )
        .await
        {
            Ok(updated) => {
                savepoint.commit().await.map_err(storage)?;
                Ok(updated)
            }
            Err(error) => {
                savepoint.rollback().await.map_err(storage)?;
                Err(error)
            }
        }
    }

    async fn apply_transition_checked_in(
        transaction: &mut Transaction<'_, Sqlite>,
        operation_id: &str,
        event: Transition,
        result_payload: Option<&[u8]>,
        now_micros: i64,
        expected_claim: Option<&Operation>,
    ) -> Result<StoredOperation, Error> {
        if result_payload.is_some_and(|value| value.len() > MAX_PAYLOAD_BYTES) || now_micros < 0 {
            return Err(Error::InvalidRecord);
        }
        let row = sqlx::query(SELECT_OPERATION_BY_ID)
            .bind(operation_id)
            .fetch_optional(&mut **transaction)
            .await
            .map_err(storage)?;
        let mut current = row
            .map(decode_operation)
            .transpose()?
            .ok_or(Error::NotFound)?;
        if let Some(claim) = expected_claim
            && (!matches_claim(&current.operation, claim)
                || claim
                    .lease_expiry_micros
                    .is_none_or(|expiry| expiry <= now_micros))
        {
            return Err(Error::ConcurrentModification);
        }
        let before = current.operation.state;
        let previous_attempt = current.operation.attempt;
        transition(&mut current.operation, event)?;
        let resolution = (current.operation.state == OperationState::Resolved)
            .then(|| current.operation.error_code.clone())
            .flatten();
        let sql = if expected_claim.is_some() {
            "UPDATE _common_operations SET state=?,attempt=?,not_before_micros=?,lease_owner=?,\
             lease_expiry_micros=?,error_code=?,resolution_code=?,result_payload=COALESCE(?,result_payload),\
             updated_at_micros=? WHERE operation_id=? AND state=? AND attempt=? \
             AND lease_owner=? AND lease_expiry_micros=? AND lease_expiry_micros>?"
        } else {
            "UPDATE _common_operations SET state=?,attempt=?,not_before_micros=?,lease_owner=?,\
             lease_expiry_micros=?,error_code=?,resolution_code=?,result_payload=COALESCE(?,result_payload),\
             updated_at_micros=? WHERE operation_id=? AND state=? AND attempt=?"
        };
        let mut query = sqlx::query(sql)
            .bind(current.operation.state.as_str())
            .bind(i64::from(current.operation.attempt))
            .bind(current.operation.not_before_micros)
            .bind(&current.operation.lease_owner)
            .bind(current.operation.lease_expiry_micros)
            .bind(&current.operation.error_code)
            .bind(resolution)
            .bind(result_payload)
            .bind(now_micros)
            .bind(operation_id)
            .bind(before.as_str())
            .bind(i64::from(
                expected_claim.map_or(previous_attempt, |claim| claim.attempt),
            ));
        if let Some(claim) = expected_claim {
            query = query
                .bind(&claim.lease_owner)
                .bind(claim.lease_expiry_micros)
                .bind(now_micros);
        }
        let done = query.execute(&mut **transaction).await.map_err(storage)?;
        if done.rows_affected() != 1 {
            return Err(Error::ConcurrentModification);
        }
        write_audit(
            transaction,
            operation_id,
            before,
            current.operation.state,
            now_micros,
        )
        .await?;
        get_by_id_in(transaction, operation_id)
            .await?
            .ok_or(Error::ConcurrentModification)
    }

    pub async fn recover_running(
        &self,
        namespace: &str,
        code: &str,
        now_micros: i64,
    ) -> Result<u64, Error> {
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT operation_id FROM _common_operations WHERE namespace=? AND state='running' \
             ORDER BY operation_id",
        )
        .bind(namespace)
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?;
        let mut recovered = 0_u64;
        for operation_id in ids {
            match self
                .apply_transition(
                    &operation_id,
                    Transition::MarkIndeterminate {
                        code: code.to_owned(),
                    },
                    None,
                    now_micros,
                )
                .await
            {
                Ok(_) => recovered += 1,
                Err(Error::ConcurrentModification | Error::InvalidTransition { .. }) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(recovered)
    }

    /// Bounded live recovery: expiry is ambiguous, never evidence of remote failure.
    pub async fn recover_expired(&self, namespace: &str, now_micros: i64) -> Result<u64, Error> {
        if namespace.is_empty() || now_micros < 0 {
            return Err(Error::InvalidRecord);
        }
        let rows = sqlx::query("SELECT * FROM _common_operations WHERE namespace=? AND state='running' AND lease_expiry_micros<=? ORDER BY lease_expiry_micros,operation_id LIMIT 128")
            .bind(namespace).bind(now_micros).fetch_all(&self.pool).await.map_err(storage)?;
        let mut recovered = 0;
        for row in rows {
            let claim = decode_operation(row)?;
            match self
                .abandon_claim(&claim.operation, "lease_expired", now_micros)
                .await
            {
                Ok(_) => recovered += 1,
                Err(Error::ConcurrentModification) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(recovered)
    }

    pub async fn pending_audit_events(&self, limit: u32) -> Result<Vec<AuditEvent>, Error> {
        if limit == 0 || limit > 1024 {
            return Err(Error::InvalidRecord);
        }
        let rows = sqlx::query(
            "SELECT event_id,operation_id,from_state,to_state,payload_json,created_at_micros \
             FROM _common_operation_audit_outbox WHERE delivered_at_micros IS NULL \
             ORDER BY created_at_micros,event_id LIMIT ?",
        )
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?;
        rows.into_iter()
            .map(|row| {
                Ok(AuditEvent {
                    event_id: row.try_get("event_id").map_err(storage)?,
                    operation_id: row.try_get("operation_id").map_err(storage)?,
                    from_state: row.try_get("from_state").map_err(storage)?,
                    to_state: row.try_get("to_state").map_err(storage)?,
                    payload_json: row.try_get("payload_json").map_err(storage)?,
                    created_at_micros: row.try_get("created_at_micros").map_err(storage)?,
                })
            })
            .collect()
    }

    pub async fn mark_audit_delivered(
        &self,
        event_id: &str,
        delivered_at_micros: i64,
    ) -> Result<bool, Error> {
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        let delivered =
            Self::mark_audit_delivered_in(&mut transaction, event_id, delivered_at_micros).await?;
        transaction.commit().await.map_err(storage)?;
        Ok(delivered)
    }

    /// Acknowledge in the same transaction as the product's idempotent audit sink.
    pub async fn mark_audit_delivered_in(
        transaction: &mut Transaction<'_, Sqlite>,
        event_id: &str,
        delivered_at_micros: i64,
    ) -> Result<bool, Error> {
        if event_id.is_empty() || delivered_at_micros < 0 {
            return Err(Error::InvalidRecord);
        }
        let result = sqlx::query(
            "UPDATE _common_operation_audit_outbox SET delivered_at_micros=? \
             WHERE event_id=? AND delivered_at_micros IS NULL",
        )
        .bind(delivered_at_micros)
        .bind(event_id)
        .execute(&mut **transaction)
        .await
        .map_err(storage)?;
        Ok(result.rows_affected() == 1)
    }
}

async fn write_audit(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    operation_id: &str,
    from: OperationState,
    to: OperationState,
    now_micros: i64,
) -> Result<(), Error> {
    write_audit_raw(
        transaction,
        operation_id,
        from.as_str(),
        to.as_str(),
        now_micros,
    )
    .await
}

async fn write_audit_raw(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    operation_id: &str,
    from: &str,
    to: &str,
    now_micros: i64,
) -> Result<(), Error> {
    let payload = serde_json::json!({"from":from,"to":to}).to_string();
    sqlx::query(
        "INSERT INTO _common_operation_audit_outbox(\
         event_id,operation_id,from_state,to_state,payload_json,created_at_micros\
         ) VALUES(?,?,?,?,?,?)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(operation_id)
    .bind(from)
    .bind(to)
    .bind(payload)
    .bind(now_micros)
    .execute(&mut **transaction)
    .await
    .map_err(storage)?;
    Ok(())
}

const SELECT_OPERATION_BY_ID: &str = "SELECT operation_id,namespace,target_key,action,idempotency_digest,request_fingerprint,request_payload,result_payload,state,attempt,max_attempts,not_before_micros,lease_owner,lease_expiry_micros,error_code,resolution_code,created_at_micros,updated_at_micros FROM _common_operations WHERE operation_id=?";
const SELECT_OPERATION_BY_IDEMPOTENCY: &str = "SELECT operation_id,namespace,target_key,action,idempotency_digest,request_fingerprint,request_payload,result_payload,state,attempt,max_attempts,not_before_micros,lease_owner,lease_expiry_micros,error_code,resolution_code,created_at_micros,updated_at_micros FROM _common_operations WHERE namespace=? AND idempotency_digest=?";
const SELECT_LATEST_OPERATION_FOR_TARGET: &str = "SELECT operation_id,namespace,target_key,action,idempotency_digest,request_fingerprint,request_payload,result_payload,state,attempt,max_attempts,not_before_micros,lease_owner,lease_expiry_micros,error_code,resolution_code,created_at_micros,updated_at_micros FROM _common_operations WHERE namespace=? AND target_key=? ORDER BY created_at_micros DESC,operation_id DESC LIMIT 1";

async fn get_by_id_in(
    transaction: &mut Transaction<'_, Sqlite>,
    operation_id: &str,
) -> Result<Option<StoredOperation>, Error> {
    let row = sqlx::query(SELECT_OPERATION_BY_ID)
        .bind(operation_id)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(storage)?;
    row.map(decode_operation).transpose()
}

async fn get_by_idempotency_in(
    transaction: &mut Transaction<'_, Sqlite>,
    namespace: &str,
    digest: &[u8; 32],
) -> Result<Option<StoredOperation>, Error> {
    let row = sqlx::query(SELECT_OPERATION_BY_IDEMPOTENCY)
        .bind(namespace)
        .bind(digest.as_slice())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(storage)?;
    row.map(decode_operation).transpose()
}

fn decode_operation(row: sqlx::sqlite::SqliteRow) -> Result<StoredOperation, Error> {
    let digest: Vec<u8> = row.try_get("idempotency_digest").map_err(storage)?;
    let fingerprint: Vec<u8> = row.try_get("request_fingerprint").map_err(storage)?;
    let idempotency_digest = digest.try_into().map_err(|_| Error::InvalidRecord)?;
    let request_fingerprint = fingerprint.try_into().map_err(|_| Error::InvalidRecord)?;
    let attempt: i64 = row.try_get("attempt").map_err(storage)?;
    let max_attempts: i64 = row.try_get("max_attempts").map_err(storage)?;
    let operation = Operation {
        operation_id: row.try_get("operation_id").map_err(storage)?,
        namespace: row.try_get("namespace").map_err(storage)?,
        target_key: row.try_get("target_key").map_err(storage)?,
        idempotency_digest,
        request_fingerprint,
        state: OperationState::parse(&row.try_get::<String, _>("state").map_err(storage)?)?,
        attempt: u32::try_from(attempt).map_err(|_| Error::InvalidRecord)?,
        max_attempts: u32::try_from(max_attempts).map_err(|_| Error::InvalidRecord)?,
        not_before_micros: row.try_get("not_before_micros").map_err(storage)?,
        lease_owner: row.try_get("lease_owner").map_err(storage)?,
        lease_expiry_micros: row.try_get("lease_expiry_micros").map_err(storage)?,
        error_code: row.try_get("error_code").map_err(storage)?,
    };
    if operation.max_attempts == 0 || operation.attempt > operation.max_attempts {
        return Err(Error::InvalidRecord);
    }
    Ok(StoredOperation {
        operation,
        action: row.try_get("action").map_err(storage)?,
        request_payload: row.try_get("request_payload").map_err(storage)?,
        result_payload: row.try_get("result_payload").map_err(storage)?,
        resolution_code: row.try_get("resolution_code").map_err(storage)?,
        created_at_micros: row.try_get("created_at_micros").map_err(storage)?,
        updated_at_micros: row.try_get("updated_at_micros").map_err(storage)?,
    })
}

fn is_constraint(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .is_some_and(|value| value.is_unique_violation())
}

fn storage(error: sqlx::Error) -> Error {
    Error::Storage(error.to_string())
}

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum Error {
    #[error("invalid operation transition from {state:?}: {event}")]
    InvalidTransition {
        state: OperationState,
        event: String,
    },
    #[error("operation attempt overflow")]
    AttemptOverflow,
    #[error("idempotency digest does not identify this operation")]
    IdempotencyKeyMismatch,
    #[error("same idempotency key was used with a different request")]
    IdempotencyConflict,
    #[error("invalid stored operation state")]
    InvalidStoredState,
    #[error("invalid durable operation record")]
    InvalidRecord,
    #[error("durable operation was not found")]
    NotFound,
    #[error("durable operation changed concurrently")]
    ConcurrentModification,
    #[error("durable operation storage failed: {0}")]
    Storage(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pending() -> Operation {
        Operation {
            operation_id: "o".into(),
            namespace: "n".into(),
            target_key: "t".into(),
            idempotency_digest: [1; 32],
            request_fingerprint: [2; 32],
            state: OperationState::Pending,
            attempt: 0,
            max_attempts: 2,
            not_before_micros: 0,
            lease_owner: None,
            lease_expiry_micros: None,
            error_code: None,
        }
    }
    #[test]
    fn unknown_is_never_retried() {
        let mut op = pending();
        transition(
            &mut op,
            Transition::Claim {
                owner: "w".into(),
                lease_expiry_micros: 1,
            },
        )
        .unwrap();
        transition(
            &mut op,
            Transition::MarkIndeterminate {
                code: "timeout".into(),
            },
        )
        .unwrap();
        assert_eq!(op.state, OperationState::Unknown);
        assert!(
            transition(
                &mut op,
                Transition::Claim {
                    owner: "w".into(),
                    lease_expiry_micros: 2
                }
            )
            .is_err()
        );
    }
    #[test]
    fn retry_budget_dead_letters() {
        let mut op = pending();
        for expected in [OperationState::Pending, OperationState::DeadLetter] {
            transition(
                &mut op,
                Transition::Claim {
                    owner: "w".into(),
                    lease_expiry_micros: 10,
                },
            )
            .unwrap();
            transition(
                &mut op,
                Transition::Fail {
                    code: "temporary".into(),
                    retryable: true,
                    retry_not_before_micros: 1,
                },
            )
            .unwrap();
            assert_eq!(op.state, expected);
        }
    }

    #[test]
    fn successful_retry_clears_the_previous_transition_error() {
        let mut operation = pending();
        transition(
            &mut operation,
            Transition::Claim {
                owner: "worker".into(),
                lease_expiry_micros: 10,
            },
        )
        .unwrap();
        transition(
            &mut operation,
            Transition::Fail {
                code: "media_request_failed".into(),
                retryable: true,
                retry_not_before_micros: 4,
            },
        )
        .unwrap();
        assert_eq!(
            operation.error_code.as_deref(),
            Some("media_request_failed")
        );
        transition(
            &mut operation,
            Transition::Claim {
                owner: "worker".into(),
                lease_expiry_micros: 12,
            },
        )
        .unwrap();
        transition(&mut operation, Transition::Succeed).unwrap();
        assert_eq!(operation.state, OperationState::Succeeded);
        assert_eq!(operation.error_code, None);
        assert_eq!(operation.lease_owner, None);
        assert_eq!(operation.lease_expiry_micros, None);
    }

    async fn store() -> SqliteOperationStore {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(PLATFORM_OPERATIONS_DDL)
            .execute(&pool)
            .await
            .unwrap();
        SqliteOperationStore::new(pool)
    }

    fn new_operation(id: &str, target: &str, fingerprint: u8) -> NewOperation {
        NewOperation {
            operation_id: id.into(),
            namespace: "media".into(),
            target_key: target.into(),
            action: "reconcile".into(),
            idempotency_digest: [id.as_bytes()[0]; 32],
            request_fingerprint: [fingerprint; 32],
            request_payload: b"{}".to_vec(),
            max_attempts: 2,
            not_before_micros: 1,
            created_at_micros: 1,
        }
    }

    #[tokio::test]
    async fn successful_retry_clears_the_previous_attempt_error() {
        let store = store().await;
        store
            .enqueue(new_operation("a", "camera", 1))
            .await
            .unwrap();
        let first = store
            .claim_next("media", "worker", 2, 10)
            .await
            .unwrap()
            .unwrap();
        let retry = store
            .apply_transition_owned(
                &first.operation,
                Transition::Fail {
                    code: "media_request_failed".into(),
                    retryable: true,
                    retry_not_before_micros: 4,
                },
                Some(br#"{"error":"media request failed"}"#),
                3,
            )
            .await
            .unwrap();
        assert_eq!(
            retry.operation.error_code.as_deref(),
            Some("media_request_failed")
        );
        let second = store
            .claim_next("media", "worker", 4, 12)
            .await
            .unwrap()
            .unwrap();
        let succeeded = store
            .apply_transition_owned(
                &second.operation,
                Transition::Succeed,
                Some(br#"{"converged":true}"#),
                5,
            )
            .await
            .unwrap();
        assert_eq!(succeeded.operation.state, OperationState::Succeeded);
        assert_eq!(succeeded.operation.attempt, 2);
        assert_eq!(succeeded.operation.error_code, None);
        assert_eq!(
            succeeded.result_payload.as_deref(),
            Some(br#"{"converged":true}"#.as_slice())
        );
        let persisted = store.get("a").await.unwrap().unwrap();
        assert_eq!(persisted, succeeded);
    }

    #[tokio::test]
    async fn sqlite_store_enforces_idempotency_and_unknown_serialization() {
        let store = store().await;
        assert_eq!(store.pending_audit_count().await.unwrap(), 0);
        assert_eq!(store.active_operation_count().await.unwrap(), 0);
        assert!(matches!(
            store
                .enqueue(new_operation("a", "camera", 1))
                .await
                .unwrap(),
            EnqueueOutcome::Created(_)
        ));
        assert!(matches!(
            store
                .enqueue(new_operation("a2", "camera", 1))
                .await
                .unwrap(),
            EnqueueOutcome::Existing(_)
        ));
        assert_eq!(
            store.enqueue(new_operation("a3", "camera", 2)).await,
            Err(Error::IdempotencyConflict)
        );
        store
            .enqueue(new_operation("b", "camera", 1))
            .await
            .unwrap();
        let claimed = store
            .claim_next("media", "worker", 2, 10)
            .await
            .unwrap()
            .unwrap();
        store
            .apply_transition(
                &claimed.operation.operation_id,
                Transition::MarkIndeterminate {
                    code: "ambiguous".into(),
                },
                None,
                3,
            )
            .await
            .unwrap();
        assert!(
            store
                .claim_next("media", "worker", 4, 12)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(store.active_operation_count().await.unwrap(), 2);
        assert_eq!(store.pending_audit_count().await.unwrap(), 4);
    }

    #[tokio::test]
    async fn product_transaction_rollback_removes_intent_and_audit_together() {
        let store = store().await;
        let mut transaction = store.pool.begin().await.unwrap();
        SqliteOperationStore::enqueue_in(&mut transaction, new_operation("a", "camera", 1))
            .await
            .unwrap();
        transaction.rollback().await.unwrap();
        assert!(store.get("a").await.unwrap().is_none());
        assert_eq!(store.pending_audit_count().await.unwrap(), 0);
        let mut transaction = store.pool.begin().await.unwrap();
        SqliteOperationStore::enqueue_in(&mut transaction, new_operation("a", "camera", 1))
            .await
            .unwrap();
        transaction.commit().await.unwrap();
        assert!(store.get("a").await.unwrap().is_some());
        assert_eq!(store.pending_audit_count().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn enqueue_waits_for_writer_before_reading_idempotency() {
        let directory = tempfile::tempdir().unwrap();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(directory.path().join("operations.db"))
                    .create_if_missing(true)
                    .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                    .busy_timeout(std::time::Duration::from_secs(5)),
            )
            .await
            .unwrap();
        sqlx::raw_sql(PLATFORM_OPERATIONS_DDL)
            .execute(&pool)
            .await
            .unwrap();
        let store = SqliteOperationStore::new(pool);
        let mut writer = store.pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        SqliteOperationStore::enqueue_in(&mut writer, new_operation("a", "camera", 1))
            .await
            .unwrap();
        // `a2` has the same idempotency digest and request fingerprint as `a`,
        // while keeping a distinct operation ID. The blocked enqueue must see
        // the committed row after acquiring the writer lock.
        let mut pending = std::pin::pin!(store.enqueue(new_operation("a2", "camera", 1)));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), &mut pending)
                .await
                .is_err()
        );
        writer.commit().await.unwrap();
        assert!(matches!(
            pending.await.unwrap(),
            EnqueueOutcome::Existing(_)
        ));
        assert_eq!(store.pending_audit_count().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn completion_requires_current_owner_and_unexpired_lease() {
        let store = store().await;
        store
            .enqueue(new_operation("a", "camera", 1))
            .await
            .unwrap();
        let claim = store
            .claim_next("media", "owner", 2, 10)
            .await
            .unwrap()
            .unwrap();
        assert!(
            store
                .apply_transition_owned(
                    &Operation {
                        lease_owner: Some("other".into()),
                        ..claim.operation.clone()
                    },
                    Transition::Succeed,
                    Some(b"{}"),
                    3
                )
                .await
                .is_err()
        );
        assert!(
            store
                .apply_transition_owned(&claim.operation, Transition::Succeed, Some(b"{}"), 10)
                .await
                .is_err()
        );
        let current = store.get("a").await.unwrap().unwrap();
        assert_eq!(current.operation.state, OperationState::Running);
        assert_eq!(store.pending_audit_count().await.unwrap(), 2);
        store
            .apply_transition_owned(&claim.operation, Transition::Succeed, Some(b"{}"), 3)
            .await
            .unwrap();
        assert_eq!(store.active_operation_count().await.unwrap(), 0);
        assert_eq!(store.pending_audit_count().await.unwrap(), 3);
    }

    #[tokio::test]
    async fn completion_and_audit_ack_join_product_transactions() {
        let store = store().await;
        store
            .enqueue(new_operation("a", "camera", 1))
            .await
            .unwrap();
        let claim = store
            .claim_next("media", "owner", 2, 10)
            .await
            .unwrap()
            .unwrap();
        let mut tx = store.pool.begin().await.unwrap();
        let updated = SqliteOperationStore::apply_transition_owned_in(
            &mut tx,
            &claim.operation,
            Transition::Succeed,
            Some(b"result"),
            3,
        )
        .await
        .unwrap();
        assert_eq!(
            updated.result_payload.as_deref(),
            Some(b"result".as_slice())
        );
        tx.rollback().await.unwrap();
        assert_eq!(
            store.get("a").await.unwrap().unwrap().operation.state,
            OperationState::Running
        );
        assert_eq!(store.pending_audit_count().await.unwrap(), 2);

        let event = store.pending_audit_events(1).await.unwrap().remove(0);
        let mut tx = store.pool.begin().await.unwrap();
        assert!(
            SqliteOperationStore::mark_audit_delivered_in(&mut tx, &event.event_id, 4)
                .await
                .unwrap()
        );
        tx.rollback().await.unwrap();
        assert_eq!(store.pending_audit_count().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn audit_failure_rolls_back_transition_even_if_caller_commits() {
        let store = store().await;
        store
            .enqueue(new_operation("a", "camera", 1))
            .await
            .unwrap();
        let claim = store
            .claim_next("media", "owner", 2, 10)
            .await
            .unwrap()
            .unwrap();
        sqlx::raw_sql("CREATE TRIGGER reject_audit BEFORE INSERT ON _common_operation_audit_outbox BEGIN SELECT RAISE(FAIL, 'injected'); END;")
            .execute(&store.pool).await.unwrap();
        let mut tx = store.pool.begin().await.unwrap();
        assert!(
            SqliteOperationStore::apply_transition_owned_in(
                &mut tx,
                &claim.operation,
                Transition::Succeed,
                Some(b"result"),
                3,
            )
            .await
            .is_err()
        );
        tx.commit().await.unwrap();
        let current = store.get("a").await.unwrap().unwrap();
        assert_eq!(current.operation.state, OperationState::Running);
        assert!(current.result_payload.is_none());
        assert_eq!(store.pending_audit_count().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn abandoned_claim_is_fenced_by_attempt_and_never_replayed() {
        let store = store().await;
        store
            .enqueue(new_operation("a", "camera", 1))
            .await
            .unwrap();
        let first = store
            .claim_next("media", "owner", 2, 10)
            .await
            .unwrap()
            .unwrap();
        store
            .apply_transition_owned(
                &first.operation,
                Transition::Fail {
                    code: "rejected".into(),
                    retryable: true,
                    retry_not_before_micros: 3,
                },
                None,
                3,
            )
            .await
            .unwrap();
        let second = store
            .claim_next("media", "owner", 4, 20)
            .await
            .unwrap()
            .unwrap();
        let audit_before = store.pending_audit_count().await.unwrap();
        assert_eq!(
            store
                .apply_transition_owned(&first.operation, Transition::Succeed, Some(b"stale"), 5)
                .await,
            Err(Error::ConcurrentModification)
        );
        assert_eq!(store.get("a").await.unwrap().unwrap(), second);
        assert_eq!(store.pending_audit_count().await.unwrap(), audit_before);
        assert_eq!(
            store.abandon_claim(&first.operation, "ambiguous", 21).await,
            Err(Error::ConcurrentModification)
        );
        let abandoned = store
            .abandon_claim(&second.operation, "ambiguous", 21)
            .await
            .unwrap();
        assert_eq!(abandoned.operation.state, OperationState::Unknown);
        assert!(
            store
                .claim_next("media", "owner", 22, 30)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn claim_rollback_and_expiry_recovery_preserve_no_replay() {
        let store = store().await;
        store
            .enqueue(new_operation("a", "camera", 1))
            .await
            .unwrap();
        let mut tx = store.pool.begin().await.unwrap();
        SqliteOperationStore::claim_next_in(&mut tx, "media", "worker", 2, 10)
            .await
            .unwrap()
            .unwrap();
        tx.rollback().await.unwrap();
        assert_eq!(store.get("a").await.unwrap().unwrap().operation.attempt, 0);
        assert_eq!(store.pending_audit_count().await.unwrap(), 1);
        store
            .claim_next("media", "worker", 2, 10)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(store.recover_expired("media", 9).await.unwrap(), 0);
        assert_eq!(store.recover_expired("media", 10).await.unwrap(), 1);
        assert_eq!(store.recover_expired("media", 11).await.unwrap(), 0);
        assert!(
            store
                .claim_next("media", "worker", 11, 20)
                .await
                .unwrap()
                .is_none()
        );
        let mut tx = store.pool.begin().await.unwrap();
        let resolved =
            SqliteOperationStore::resolve_in(&mut tx, "a", Resolution::UnableToConfirm, 12)
                .await
                .unwrap();
        assert_eq!(resolved.operation.state, OperationState::DeadLetter);
        tx.rollback().await.unwrap();
        assert_eq!(
            store.get("a").await.unwrap().unwrap().operation.state,
            OperationState::Unknown
        );
        let mut tx = store.pool.begin().await.unwrap();
        SqliteOperationStore::resolve_in(&mut tx, "a", Resolution::ConfirmedSucceeded, 12)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(
            store
                .get("a")
                .await
                .unwrap()
                .unwrap()
                .resolution_code
                .as_deref(),
            Some("confirmed_succeeded")
        );
        assert!(
            store
                .claim_next("media", "worker", 13, 20)
                .await
                .unwrap()
                .is_none()
        );
    }
}
