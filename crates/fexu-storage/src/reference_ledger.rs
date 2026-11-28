//! Opt-in durable admission gate for cooperating application writers.

use crate::{MediaRef, ReferenceCheck, ReferenceDecision, StorageError};
use async_trait::async_trait;
use fexu_db::driver::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, DatabaseTransaction, Statement,
    TransactionTrait, Value,
};
use sha2::{Digest, Sha256};

/// Counts model references in the same transaction as each model write.
/// Retirement is permanent, including when a subsequent storage request fails.
/// This is not a scan of existing models or a replacement for authorization.
#[derive(Clone)]
pub struct ReferenceLedger {
    db: DatabaseConnection,
}

/// Outcome of one explicitly abandoned immutable ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbandonedCleanup {
    Unknown,
    Referenced,
    Removed,
    Missing,
}

/// One bounded worker invocation. A cursor is a progress hint, not a snapshot.
/// Restart from None after reaching the end to retry failed or newly eligible IDs.
#[derive(Debug)]
pub struct CleanupBatch {
    pub items: Vec<AbandonedCleanupItem>,
    pub next_after: Option<MediaRef>,
    pub reached_end: bool,
    pub scan_error: Option<StorageError>,
}

/// Bounded local quota sweep progress. False means no new charge was released.
#[derive(Debug)]
pub struct LocalQuotaCleanupBatch {
    pub items: Vec<LocalQuotaCleanupItem>,
    pub next_after: Option<MediaRef>,
    pub reached_end: bool,
    pub scan_error: Option<StorageError>,
}

/// One attempted local reservation release.
#[derive(Debug)]
pub struct LocalQuotaCleanupItem {
    pub media: MediaRef,
    pub outcome: Result<bool, StorageError>,
}

/// Recorded backend result, not evidence of physical erasure or backup removal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupReceipt {
    Removed,
    Missing,
}

/// Per-ID result of a bounded cleanup page. Errors remain retryable.
#[derive(Debug)]
pub struct AbandonedCleanupItem {
    pub media: MediaRef,
    pub outcome: Result<AbandonedCleanup, StorageError>,
}

/// Durable write status for uploads enrolled through `stage_reserved`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadState {
    Uploading,
    Staged,
    Committed,
    Abandoned,
}

/// Conservative logical reservation budget, not filesystem usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnerQuota {
    pub max_files: i64,
    pub max_bytes: i64,
    pub reserved_files: i64,
    pub reserved_bytes: i64,
}

/// Read-only comparison of a logical budget with its charge journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnerQuotaAudit {
    pub quota: OwnerQuota,
    pub journaled_files: i64,
    pub journaled_bytes: i64,
    pub released_files: i64,
    pub unjournaled_uploads: i64,
    pub legacy_references: i64,
}

impl OwnerQuotaAudit {
    /// A positive gap is not proof of a releasable historical reservation.
    pub fn counters_match_journal(&self) -> bool {
        self.quota.reserved_files == self.journaled_files
            && self.quota.reserved_bytes == self.journaled_bytes
    }
}

/// Immutable expected upload properties, recorded before storage I/O.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadIntent {
    pub size: i64,
    pub sha256: String,
    pub expires_at: i64,
    pub original_name: String,
}

impl UploadIntent {
    fn new(original_name: &str, bytes: &[u8], expires_at: i64) -> Result<Self, StorageError> {
        let size = i64::try_from(bytes.len()).map_err(|_| StorageError::TooLarge)?;
        Ok(Self {
            size,
            sha256: crate::local::hex(Sha256::digest(bytes)),
            expires_at,
            original_name: crate::sanitize_name(original_name),
        })
    }

    fn matches(&self, file: &crate::StoredFile) -> bool {
        u64::try_from(self.size).ok() == Some(file.size)
            && self.sha256 == file.sha256
            && Some(self.expires_at) == file.expires_at
            && self.original_name == file.original_name
    }
}

/// Non-authoritative observation of an unresolved write. No variant
/// establishes that a delayed write cannot still complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadObservation {
    NotUploading,
    Present(crate::StoredFile),
    Missing,
    Expired,
}

/// An attempted reserved upload. Do not infer registration or write success
/// from an error; inspect the ledger and storage before recovery.
#[derive(Debug)]
pub struct ReservedUploadError {
    pub media: MediaRef,
    pub error: StorageError,
}

fn statement(
    db: &impl ConnectionTrait,
    sqlite: &str,
    postgres: &str,
    media: MediaRef,
) -> Result<Statement, StorageError> {
    let backend = db.get_database_backend();
    let sql = match backend {
        DatabaseBackend::Sqlite => sqlite,
        DatabaseBackend::Postgres => postgres,
        DatabaseBackend::MySql => return Err(StorageError::Failed),
    };
    Ok(Statement::from_sql_and_values(
        backend,
        sql,
        [
            Value::from(media.owner.to_string()),
            Value::from(media.id.to_string()),
        ],
    ))
}

impl ReferenceLedger {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    pub(crate) fn database(&self) -> &DatabaseConnection {
        &self.db
    }

    /// Explicit setup only; no automatic runtime schema mutation.
    pub async fn initialize(&self) -> Result<(), StorageError> {
        self.db
            .execute_unprepared(
                "CREATE TABLE IF NOT EXISTS _fexu_media_references (
                    owner TEXT NOT NULL,
                    file_id TEXT NOT NULL,
                    refs BIGINT NOT NULL CHECK (refs >= 0),
                    retired INTEGER NOT NULL CHECK (retired IN (0, 1)),
                    PRIMARY KEY (owner, file_id),
                    CHECK (retired = 0 OR refs = 0)
                )",
            )
            .await
            .map_err(|_| StorageError::Failed)?;
        self.db
            .execute_unprepared(
                "CREATE TABLE IF NOT EXISTS _fexu_media_uploads (
                    owner TEXT NOT NULL,
                    file_id TEXT NOT NULL,
                    state INTEGER NOT NULL CHECK (state IN (0, 1, 2, 3)),
                    PRIMARY KEY (owner, file_id)
                )",
            )
            .await
            .map_err(|_| StorageError::Failed)?;
        self.db
            .execute_unprepared(
                "CREATE TABLE IF NOT EXISTS _fexu_media_cleanup_receipts (
                    owner TEXT NOT NULL,
                    file_id TEXT NOT NULL,
                    outcome INTEGER NOT NULL CHECK (outcome IN (1, 2)),
                    PRIMARY KEY (owner, file_id)
                )",
            )
            .await
            .map_err(|_| StorageError::Failed)?;
        self.db
            .execute_unprepared(
                "CREATE TABLE IF NOT EXISTS _fexu_media_upload_intents (
                    owner TEXT NOT NULL,
                    file_id TEXT NOT NULL,
                    size BIGINT NOT NULL CHECK (size >= 0),
                    sha256 TEXT NOT NULL,
                    expires_at BIGINT NOT NULL,
                    original_name TEXT NOT NULL,
                    PRIMARY KEY (owner, file_id)
                )",
            )
            .await
            .map_err(|_| StorageError::Failed)?;
        self.db
            .execute_unprepared(
                "CREATE TABLE IF NOT EXISTS _fexu_media_upload_fences (
                    owner TEXT NOT NULL,
                    file_id TEXT NOT NULL,
                    PRIMARY KEY (owner, file_id)
                )",
            )
            .await
            .map_err(|_| StorageError::Failed)?;
        self.db
            .execute_unprepared(
                "CREATE TABLE IF NOT EXISTS _fexu_media_local_fences (
                    owner TEXT NOT NULL,
                    file_id TEXT NOT NULL,
                    PRIMARY KEY (owner, file_id)
                )",
            )
            .await
            .map_err(|_| StorageError::Failed)?;
        self.db
            .execute_unprepared(
                "CREATE TABLE IF NOT EXISTS _fexu_media_owner_quotas (
                    owner TEXT PRIMARY KEY NOT NULL,
                    max_files BIGINT NOT NULL CHECK (max_files >= 0),
                    max_bytes BIGINT NOT NULL CHECK (max_bytes >= 0),
                    reserved_files BIGINT NOT NULL CHECK (reserved_files >= 0),
                    reserved_bytes BIGINT NOT NULL CHECK (reserved_bytes >= 0),
                    CHECK (reserved_files <= max_files AND reserved_bytes <= max_bytes)
                )",
            )
            .await
            .map_err(|_| StorageError::Failed)?;
        self.db
            .execute_unprepared(
                "CREATE TABLE IF NOT EXISTS _fexu_media_quota_charges (
                    owner TEXT NOT NULL,
                    file_id TEXT NOT NULL,
                    size BIGINT NOT NULL CHECK (size >= 0),
                    released INTEGER NOT NULL CHECK (released IN (0, 1)),
                    PRIMARY KEY (owner, file_id)
                )",
            )
            .await
            .map_err(|_| StorageError::Failed)?;
        Ok(())
    }

    /// Enroll a newly allocated immutable ID before its first reference.
    /// An existing or retired ID must never be registered again.
    pub async fn register(&self, media: MediaRef) -> Result<(), StorageError> {
        register_on(&self.db, media).await
    }

    /// Allocate and durably enroll an immutable ID before writing bytes.
    /// The caller must authorize the upload. Failed or ambiguous writes stay
    /// Uploading until an operator establishes that all writes have settled.
    pub async fn stage_reserved<S: crate::Storage + ?Sized>(
        &self,
        storage: &S,
        owner: uuid::Uuid,
        original_name: &str,
        bytes: &[u8],
        expires_at: i64,
    ) -> Result<crate::StoredFile, ReservedUploadError> {
        let media = MediaRef {
            owner,
            id: uuid::Uuid::new_v4(),
        };
        let intent = UploadIntent::new(original_name, bytes, expires_at)
            .map_err(|error| ReservedUploadError { media, error })?;
        self.begin_upload(media, &intent)
            .await
            .map_err(|error| ReservedUploadError { media, error })?;
        let file = storage
            .save_reserved(media, original_name, bytes, expires_at)
            .await
            .map_err(|error| ReservedUploadError { media, error })?;
        if file.owner != media.owner || file.id != media.id || !intent.matches(&file) {
            return Err(ReservedUploadError {
                media,
                error: StorageError::Failed,
            });
        }
        self.finish_upload(media)
            .await
            .map_err(|error| ReservedUploadError { media, error })?;
        Ok(file)
    }

    // Enrollment and the in-flight marker must become visible together.
    async fn begin_upload(
        &self,
        media: MediaRef,
        intent: &UploadIntent,
    ) -> Result<(), StorageError> {
        let tx = self.db.begin().await.map_err(|_| StorageError::Failed)?;
        Self::reserve_quota(&tx, media.owner, intent.size).await?;
        register_on(&tx, media).await?;
        let query = statement(
            &tx,
            "INSERT INTO _fexu_media_uploads VALUES (?, ?, 0)",
            "INSERT INTO _fexu_media_uploads VALUES ($1, $2, 0)",
            media,
        )?;
        change_one(&tx, query).await?;
        let backend = tx.get_database_backend();
        let sql = match backend {
            DatabaseBackend::Sqlite => {
                "INSERT INTO _fexu_media_upload_intents VALUES (?, ?, ?, ?, ?, ?)"
            }
            DatabaseBackend::Postgres => {
                "INSERT INTO _fexu_media_upload_intents VALUES ($1, $2, $3, $4, $5, $6)"
            }
            _ => return Err(StorageError::Failed),
        };
        let query = Statement::from_sql_and_values(
            backend,
            sql,
            [
                Value::from(media.owner.to_string()),
                Value::from(media.id.to_string()),
                Value::from(intent.size),
                Value::from(intent.sha256.clone()),
                Value::from(intent.expires_at),
                Value::from(intent.original_name.clone()),
            ],
        );
        change_one(&tx, query).await?;
        let sql = match backend {
            DatabaseBackend::Sqlite => "INSERT INTO _fexu_media_quota_charges VALUES (?, ?, ?, 0)",
            DatabaseBackend::Postgres => {
                "INSERT INTO _fexu_media_quota_charges VALUES ($1, $2, $3, 0)"
            }
            _ => return Err(StorageError::Failed),
        };
        change_one(
            &tx,
            Statement::from_sql_and_values(
                backend,
                sql,
                [
                    Value::from(media.owner.to_string()),
                    Value::from(media.id.to_string()),
                    Value::from(intent.size),
                ],
            ),
        )
        .await?;
        tx.commit().await.map_err(|_| StorageError::Failed)
    }

    // Every tracked admission locks the same owner row, even when unlimited.
    // A sentinel budget of i64::MAX is unconfigured; counters are still charged.
    async fn lock_owner_quota(
        tx: &DatabaseTransaction,
        owner: uuid::Uuid,
    ) -> Result<(), StorageError> {
        let backend = tx.get_database_backend();
        let sql = match backend {
            DatabaseBackend::Sqlite => {
                "INSERT INTO _fexu_media_owner_quotas VALUES (?, 9223372036854775807,
                 9223372036854775807, 0, 0) ON CONFLICT (owner) DO NOTHING"
            }
            DatabaseBackend::Postgres => {
                "INSERT INTO _fexu_media_owner_quotas VALUES ($1, 9223372036854775807,
                 9223372036854775807, 0, 0) ON CONFLICT (owner) DO NOTHING"
            }
            _ => return Err(StorageError::Failed),
        };
        tx.execute(Statement::from_sql_and_values(
            backend,
            sql,
            [Value::from(owner.to_string())],
        ))
        .await
        .map_err(|_| StorageError::Failed)?;
        let sql = match backend {
            DatabaseBackend::Sqlite => {
                "UPDATE _fexu_media_owner_quotas SET max_files = max_files WHERE owner = ?"
            }
            DatabaseBackend::Postgres => {
                "UPDATE _fexu_media_owner_quotas SET max_files = max_files WHERE owner = $1"
            }
            _ => return Err(StorageError::Failed),
        };
        change_one(
            tx,
            Statement::from_sql_and_values(backend, sql, [Value::from(owner.to_string())]),
        )
        .await
    }

    async fn reserve_quota(
        tx: &DatabaseTransaction,
        owner: uuid::Uuid,
        size: i64,
    ) -> Result<(), StorageError> {
        if size < 0 {
            return Err(StorageError::Failed);
        }
        Self::lock_owner_quota(tx, owner).await?;
        let backend = tx.get_database_backend();
        let sql = match backend {
            DatabaseBackend::Sqlite => {
                "UPDATE _fexu_media_owner_quotas
                 SET reserved_files = reserved_files + 1, reserved_bytes = reserved_bytes + ?
                 WHERE owner = ? AND reserved_files < max_files
                 AND reserved_bytes <= max_bytes - ?"
            }
            DatabaseBackend::Postgres => {
                "UPDATE _fexu_media_owner_quotas
                 SET reserved_files = reserved_files + 1, reserved_bytes = reserved_bytes + $1
                 WHERE owner = $2 AND reserved_files < max_files
                 AND reserved_bytes <= max_bytes - $3"
            }
            _ => return Err(StorageError::Failed),
        };
        let changed = tx
            .execute(Statement::from_sql_and_values(
                backend,
                sql,
                [
                    Value::from(size),
                    Value::from(owner.to_string()),
                    Value::from(size),
                ],
            ))
            .await
            .map_err(|_| StorageError::Failed)?;
        match changed.rows_affected() {
            1 => Ok(()),
            0 => Err(StorageError::QuotaExceeded),
            _ => Err(StorageError::Failed),
        }
    }

    /// Configure a budget for an owner before its first ledger enrollment.
    /// Use resize_owner_quota for an existing logical budget. Adoption of
    /// legacy media requires separate reconciliation. Zero is a real limit.
    /// All writers must use reserved admission; raw and legacy saves are excluded.
    pub async fn configure_owner_quota(
        &self,
        owner: uuid::Uuid,
        max_files: i64,
        max_bytes: i64,
    ) -> Result<(), StorageError> {
        if max_files < 0 || max_bytes < 0 {
            return Err(StorageError::Failed);
        }
        let tx = self.db.begin().await.map_err(|_| StorageError::Failed)?;
        Self::lock_owner_quota(&tx, owner).await?;
        let backend = tx.get_database_backend();
        let sql = match backend {
            DatabaseBackend::Sqlite => {
                "UPDATE _fexu_media_owner_quotas SET max_files = ?, max_bytes = ?
                 WHERE owner = ? AND reserved_files = 0 AND reserved_bytes = 0
                 AND NOT EXISTS (SELECT 1 FROM _fexu_media_references r
                     WHERE r.owner = _fexu_media_owner_quotas.owner)"
            }
            DatabaseBackend::Postgres => {
                "UPDATE _fexu_media_owner_quotas SET max_files = $1, max_bytes = $2
                 WHERE owner = $3 AND reserved_files = 0 AND reserved_bytes = 0
                 AND NOT EXISTS (SELECT 1 FROM _fexu_media_references r
                     WHERE r.owner = _fexu_media_owner_quotas.owner)"
            }
            _ => return Err(StorageError::Failed),
        };
        change_one(
            &tx,
            Statement::from_sql_and_values(
                backend,
                sql,
                [
                    Value::from(max_files),
                    Value::from(max_bytes),
                    Value::from(owner.to_string()),
                ],
            ),
        )
        .await?;
        tx.commit().await.map_err(|_| StorageError::Failed)
    }

    /// Change an existing logical budget without rewriting its reservations.
    /// Only an authorized operator may call this. All writers must use reserved
    /// admission. This does not adopt legacy media or reconcile physical usage.
    /// Returns false for an absent budget or limits below current reservations.
    /// Admission, release and resizing serialize on the same owner row.
    pub async fn resize_owner_quota(
        &self,
        owner: uuid::Uuid,
        max_files: i64,
        max_bytes: i64,
    ) -> Result<bool, StorageError> {
        if max_files < 0 || max_bytes < 0 {
            return Err(StorageError::Failed);
        }
        let backend = self.db.get_database_backend();
        let sql = match backend {
            DatabaseBackend::Sqlite => {
                "UPDATE _fexu_media_owner_quotas SET max_files = ?, max_bytes = ?
                 WHERE owner = ? AND reserved_files <= ? AND reserved_bytes <= ?"
            }
            DatabaseBackend::Postgres => {
                "UPDATE _fexu_media_owner_quotas SET max_files = $1, max_bytes = $2
                 WHERE owner = $3 AND reserved_files <= $4 AND reserved_bytes <= $5"
            }
            _ => return Err(StorageError::Failed),
        };
        let changed = self
            .db
            .execute(Statement::from_sql_and_values(
                backend,
                sql,
                [
                    Value::from(max_files),
                    Value::from(max_bytes),
                    Value::from(owner.to_string()),
                    Value::from(max_files),
                    Value::from(max_bytes),
                ],
            ))
            .await
            .map_err(|_| StorageError::Failed)?;
        match changed.rows_affected() {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(StorageError::Failed),
        }
    }

    /// Read conservative reservation counters. Cleanup does not automatically
    /// release them: a backend receipt alone cannot prove residue is gone.
    pub async fn owner_quota(&self, owner: uuid::Uuid) -> Result<Option<OwnerQuota>, StorageError> {
        let backend = self.db.get_database_backend();
        let sql = match backend {
            DatabaseBackend::Sqlite => {
                "SELECT max_files, max_bytes, reserved_files, reserved_bytes
                 FROM _fexu_media_owner_quotas WHERE owner = ?"
            }
            DatabaseBackend::Postgres => {
                "SELECT max_files, max_bytes, reserved_files, reserved_bytes
                 FROM _fexu_media_owner_quotas WHERE owner = $1"
            }
            _ => return Err(StorageError::Failed),
        };
        let row = self
            .db
            .query_one(Statement::from_sql_and_values(
                backend,
                sql,
                [Value::from(owner.to_string())],
            ))
            .await
            .map_err(|_| StorageError::Failed)?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(OwnerQuota {
            max_files: row
                .try_get("", "max_files")
                .map_err(|_| StorageError::Failed)?,
            max_bytes: row
                .try_get("", "max_bytes")
                .map_err(|_| StorageError::Failed)?,
            reserved_files: row
                .try_get("", "reserved_files")
                .map_err(|_| StorageError::Failed)?,
            reserved_bytes: row
                .try_get("", "reserved_bytes")
                .map_err(|_| StorageError::Failed)?,
        }))
    }

    /// Compare an existing owner's budget and journal in one read statement.
    /// Only authorized operators may inspect owner accounting. None means
    /// no budget row, not proof of absent media. Historical counts describe
    /// missing enrollment records, not physical usage or releasable charges.
    /// This never changes counters, creates rows, or accesses storage.
    pub async fn audit_owner_quota(
        &self,
        owner: uuid::Uuid,
    ) -> Result<Option<OwnerQuotaAudit>, StorageError> {
        let backend = self.db.get_database_backend();
        let predicate = match backend {
            DatabaseBackend::Sqlite => "?",
            DatabaseBackend::Postgres => "$1",
            _ => return Err(StorageError::Failed),
        };
        let sql = format!(
            "SELECT b.max_files, b.max_bytes, b.reserved_files, b.reserved_bytes,
                (SELECT COUNT(*) FROM _fexu_media_quota_charges q
                 WHERE q.owner = b.owner AND q.released = 0) AS journaled_files,
                (SELECT CAST(COALESCE(SUM(q.size), 0) AS BIGINT)
                 FROM _fexu_media_quota_charges q
                 WHERE q.owner = b.owner AND q.released = 0) AS journaled_bytes,
                (SELECT COUNT(*) FROM _fexu_media_quota_charges q
                 WHERE q.owner = b.owner AND q.released = 1) AS released_files,
                (SELECT COUNT(*) FROM _fexu_media_uploads u
                 WHERE u.owner = b.owner AND NOT EXISTS (
                     SELECT 1 FROM _fexu_media_quota_charges q
                     WHERE q.owner = u.owner AND q.file_id = u.file_id)) AS unjournaled_uploads,
                (SELECT COUNT(*) FROM _fexu_media_references r
                 WHERE r.owner = b.owner AND NOT EXISTS (
                     SELECT 1 FROM _fexu_media_uploads u
                     WHERE u.owner = r.owner AND u.file_id = r.file_id)) AS legacy_references
             FROM _fexu_media_owner_quotas b WHERE b.owner = {predicate}"
        );
        let row = self
            .db
            .query_one(Statement::from_sql_and_values(
                backend,
                sql,
                [Value::from(owner.to_string())],
            ))
            .await
            .map_err(|_| StorageError::Failed)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let get = |name: &str| -> Result<i64, StorageError> {
            row.try_get("", name).map_err(|_| StorageError::Failed)
        };
        Ok(Some(OwnerQuotaAudit {
            quota: OwnerQuota {
                max_files: get("max_files")?,
                max_bytes: get("max_bytes")?,
                reserved_files: get("reserved_files")?,
                reserved_bytes: get("reserved_bytes")?,
            },
            journaled_files: get("journaled_files")?,
            journaled_bytes: get("journaled_bytes")?,
            released_files: get("released_files")?,
            unjournaled_uploads: get("unjournaled_uploads")?,
            legacy_references: get("legacy_references")?,
        }))
    }

    /// Adopt an existing, referenced legacy file into logical quota accounting.
    /// Authorized operators must independently establish that this ID was
    /// never charged, all writers have settled, all references are enrolled,
    /// and the original trusted storage namespace is used. Keep legacy
    /// writers, raw mutation and destructive operations offline during adoption.
    /// Only retained, integrity-checked files with a live reference qualify.
    /// No original upload intent is fabricated. No storage mutation occurs.
    /// An existing upload or charge, retired ID or missing reference returns
    /// false; insufficient capacity returns QuotaExceeded.
    pub async fn adopt_legacy_quota<S: crate::Storage + ?Sized>(
        &self,
        storage: &S,
        media: MediaRef,
    ) -> Result<bool, StorageError> {
        let (file, bytes) = storage.read(media.owner, media.id).await?;
        let size = i64::try_from(bytes.len()).map_err(|_| StorageError::TooLarge)?;
        if file.owner != media.owner
            || file.id != media.id
            || file.expires_at.is_some()
            || file.size != bytes.len() as u64
            || file.sha256 != crate::local::hex(Sha256::digest(&bytes))
        {
            return Err(StorageError::Failed);
        }
        let tx = self.db.begin().await.map_err(|_| StorageError::Failed)?;
        Self::lock_owner_quota(&tx, media.owner).await?;
        // Serialize with reference changes and retirement before checking
        // enrollment in a separate statement.
        let query = statement(
            &tx,
            "UPDATE _fexu_media_references SET refs = refs
             WHERE owner = ? AND file_id = ? AND refs > 0 AND retired = 0",
            "UPDATE _fexu_media_references SET refs = refs
             WHERE owner = $1 AND file_id = $2 AND refs > 0 AND retired = 0",
            media,
        )?;
        let changed = tx.execute(query).await.map_err(|_| StorageError::Failed)?;
        if changed.rows_affected() != 1 {
            tx.rollback().await.map_err(|_| StorageError::Failed)?;
            return Ok(false);
        }
        let query = statement(
            &tx,
            "INSERT INTO _fexu_media_uploads (owner, file_id, state)
             SELECT r.owner, r.file_id, 2 FROM _fexu_media_references r
             WHERE r.owner = ? AND r.file_id = ?
             AND NOT EXISTS (SELECT 1 FROM _fexu_media_quota_charges q
                 WHERE q.owner = r.owner AND q.file_id = r.file_id)
             AND NOT EXISTS (SELECT 1 FROM _fexu_media_upload_intents i
                 WHERE i.owner = r.owner AND i.file_id = r.file_id)
             AND NOT EXISTS (SELECT 1 FROM _fexu_media_cleanup_receipts c
                 WHERE c.owner = r.owner AND c.file_id = r.file_id)
             AND NOT EXISTS (SELECT 1 FROM _fexu_media_upload_fences f
                 WHERE f.owner = r.owner AND f.file_id = r.file_id)
             AND NOT EXISTS (SELECT 1 FROM _fexu_media_local_fences f
                 WHERE f.owner = r.owner AND f.file_id = r.file_id)
             ON CONFLICT (owner, file_id) DO NOTHING",
            "INSERT INTO _fexu_media_uploads (owner, file_id, state)
             SELECT r.owner, r.file_id, 2 FROM _fexu_media_references r
             WHERE r.owner = $1 AND r.file_id = $2
             AND NOT EXISTS (SELECT 1 FROM _fexu_media_quota_charges q
                 WHERE q.owner = r.owner AND q.file_id = r.file_id)
             AND NOT EXISTS (SELECT 1 FROM _fexu_media_upload_intents i
                 WHERE i.owner = r.owner AND i.file_id = r.file_id)
             AND NOT EXISTS (SELECT 1 FROM _fexu_media_cleanup_receipts c
                 WHERE c.owner = r.owner AND c.file_id = r.file_id)
             AND NOT EXISTS (SELECT 1 FROM _fexu_media_upload_fences f
                 WHERE f.owner = r.owner AND f.file_id = r.file_id)
             AND NOT EXISTS (SELECT 1 FROM _fexu_media_local_fences f
                 WHERE f.owner = r.owner AND f.file_id = r.file_id)
             ON CONFLICT (owner, file_id) DO NOTHING",
            media,
        )?;
        let changed = tx.execute(query).await.map_err(|_| StorageError::Failed)?;
        if changed.rows_affected() != 1 {
            tx.rollback().await.map_err(|_| StorageError::Failed)?;
            return Ok(false);
        }
        Self::reserve_quota(&tx, media.owner, size).await?;
        let backend = tx.get_database_backend();
        let sql = match backend {
            DatabaseBackend::Sqlite => "INSERT INTO _fexu_media_quota_charges VALUES (?, ?, ?, 0)",
            DatabaseBackend::Postgres => {
                "INSERT INTO _fexu_media_quota_charges VALUES ($1, $2, $3, 0)"
            }
            _ => return Err(StorageError::Failed),
        };
        change_one(
            &tx,
            Statement::from_sql_and_values(
                backend,
                sql,
                [
                    Value::from(media.owner.to_string()),
                    Value::from(media.id.to_string()),
                    Value::from(size),
                ],
            ),
        )
        .await?;
        tx.commit().await.map_err(|_| StorageError::Failed)?;
        Ok(true)
    }

    /// Journal one independently verified historical reservation.
    /// An authorized operator must prove this exact ID and original intent
    /// were already charged and never released. A counter gap alone is not
    /// evidence of that fact. Exclude legacy writers and raw database changes.
    /// No quota counters are changed. Missing intent, mismatched size, existing
    /// journals (including released ones) and insufficient gaps return false.
    pub async fn journal_historical_quota(
        &self,
        media: MediaRef,
        expected_size: i64,
    ) -> Result<bool, StorageError> {
        if expected_size < 0 {
            return Err(StorageError::Failed);
        }
        let tx = self.db.begin().await.map_err(|_| StorageError::Failed)?;
        Self::lock_owner_quota(&tx, media.owner).await?;
        let backend = tx.get_database_backend();
        let (owner, id, size) = match backend {
            DatabaseBackend::Sqlite => ("?1", "?2", "?3"),
            DatabaseBackend::Postgres => ("$1", "$2", "$3"),
            _ => return Err(StorageError::Failed),
        };
        let sql = format!(
            "INSERT INTO _fexu_media_quota_charges (owner, file_id, size, released)
             SELECT u.owner, u.file_id, i.size, 0 FROM _fexu_media_uploads u
             JOIN _fexu_media_upload_intents i ON i.owner = u.owner AND i.file_id = u.file_id
             JOIN _fexu_media_references r ON r.owner = u.owner AND r.file_id = u.file_id
             JOIN _fexu_media_owner_quotas b ON b.owner = u.owner
             WHERE u.owner = {owner} AND u.file_id = {id} AND i.size = {size}
             AND b.reserved_files > (SELECT COUNT(*) FROM _fexu_media_quota_charges q
                 WHERE q.owner = b.owner AND q.released = 0)
             AND b.reserved_bytes >= i.size
             AND b.reserved_bytes - i.size >= (SELECT COALESCE(SUM(q.size), 0)
                 FROM _fexu_media_quota_charges q WHERE q.owner = b.owner AND q.released = 0)
             ON CONFLICT (owner, file_id) DO NOTHING"
        );
        let changed = tx
            .execute(Statement::from_sql_and_values(
                backend,
                sql,
                [
                    Value::from(media.owner.to_string()),
                    Value::from(media.id.to_string()),
                    Value::from(expected_size),
                ],
            ))
            .await
            .map_err(|_| StorageError::Failed)?;
        match changed.rows_affected() {
            0 => {
                tx.rollback().await.map_err(|_| StorageError::Failed)?;
                Ok(false)
            }
            1 => {
                tx.commit().await.map_err(|_| StorageError::Failed)?;
                Ok(true)
            }
            _ => Err(StorageError::Failed),
        }
    }

    /// Release a journaled logical quota charge exactly once.
    /// Only an authorized operator may call this after confirming permanent
    /// abandonment, settlement of all writes and removal of storage residue
    /// in the original trusted namespace. A cleanup receipt is necessary but
    /// is not sufficient evidence of these conditions. No physical deletion
    /// is performed here. Older unjournaled reservations stay charged.
    /// Returns false for ineligible, unjournaled or already released IDs.
    pub async fn release_settled_quota(&self, media: MediaRef) -> Result<bool, StorageError> {
        let tx = self.db.begin().await.map_err(|_| StorageError::Failed)?;
        Self::lock_owner_quota(&tx, media.owner).await?;
        let query = statement(
            &tx,
            "SELECT q.size FROM _fexu_media_quota_charges q
             JOIN _fexu_media_references r ON r.owner = q.owner AND r.file_id = q.file_id
             JOIN _fexu_media_uploads u ON u.owner = q.owner AND u.file_id = q.file_id
             JOIN _fexu_media_cleanup_receipts c ON c.owner = q.owner AND c.file_id = q.file_id
             WHERE q.owner = ? AND q.file_id = ? AND q.released = 0
             AND r.refs = 0 AND r.retired = 1 AND u.state = 3 AND c.outcome IN (1, 2)",
            "SELECT q.size FROM _fexu_media_quota_charges q
             JOIN _fexu_media_references r ON r.owner = q.owner AND r.file_id = q.file_id
             JOIN _fexu_media_uploads u ON u.owner = q.owner AND u.file_id = q.file_id
             JOIN _fexu_media_cleanup_receipts c ON c.owner = q.owner AND c.file_id = q.file_id
             WHERE q.owner = $1 AND q.file_id = $2 AND q.released = 0
             AND r.refs = 0 AND r.retired = 1 AND u.state = 3 AND c.outcome IN (1, 2)",
            media,
        )?;
        let row = tx
            .query_one(query)
            .await
            .map_err(|_| StorageError::Failed)?;
        let Some(row) = row else {
            tx.rollback().await.map_err(|_| StorageError::Failed)?;
            return Ok(false);
        };
        let size: i64 = row.try_get("", "size").map_err(|_| StorageError::Failed)?;
        if size < 0 {
            return Err(StorageError::Failed);
        }
        let query = statement(
            &tx,
            "UPDATE _fexu_media_quota_charges SET released = 1
             WHERE owner = ? AND file_id = ? AND released = 0",
            "UPDATE _fexu_media_quota_charges SET released = 1
             WHERE owner = $1 AND file_id = $2 AND released = 0",
            media,
        )?;
        change_one(&tx, query).await?;
        let backend = tx.get_database_backend();
        let sql = match backend {
            DatabaseBackend::Sqlite => {
                "UPDATE _fexu_media_owner_quotas
                 SET reserved_files = reserved_files - 1, reserved_bytes = reserved_bytes - ?
                 WHERE owner = ? AND reserved_files > 0 AND reserved_bytes >= ?"
            }
            DatabaseBackend::Postgres => {
                "UPDATE _fexu_media_owner_quotas
                 SET reserved_files = reserved_files - 1, reserved_bytes = reserved_bytes - $1
                 WHERE owner = $2 AND reserved_files > 0 AND reserved_bytes >= $3"
            }
            _ => return Err(StorageError::Failed),
        };
        change_one(
            &tx,
            Statement::from_sql_and_values(
                backend,
                sql,
                [
                    Value::from(size),
                    Value::from(media.owner.to_string()),
                    Value::from(size),
                ],
            ),
        )
        .await?;
        tx.commit().await.map_err(|_| StorageError::Failed)?;
        Ok(true)
    }

    /// Read the original intent without changing upload state.
    /// Older enrollments may have no intent and cannot use settled recovery.
    pub async fn upload_intent(
        &self,
        media: MediaRef,
    ) -> Result<Option<UploadIntent>, StorageError> {
        let query = statement(
            &self.db,
            "SELECT size, sha256, expires_at, original_name FROM _fexu_media_upload_intents
             WHERE owner = ? AND file_id = ?",
            "SELECT size, sha256, expires_at, original_name FROM _fexu_media_upload_intents
             WHERE owner = $1 AND file_id = $2",
            media,
        )?;
        let row = self
            .db
            .query_one(query)
            .await
            .map_err(|_| StorageError::Failed)?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(UploadIntent {
            size: row.try_get("", "size").map_err(|_| StorageError::Failed)?,
            sha256: row
                .try_get("", "sha256")
                .map_err(|_| StorageError::Failed)?,
            expires_at: row
                .try_get("", "expires_at")
                .map_err(|_| StorageError::Failed)?,
            original_name: row
                .try_get("", "original_name")
                .map_err(|_| StorageError::Failed)?,
        }))
    }

    // First acquire the upload row's write lock, then read the fence request
    // in a separate statement (PostgreSQL READ COMMITTED / SQLite).
    async fn lock_upload(tx: &DatabaseTransaction, media: MediaRef) -> Result<(), StorageError> {
        let query = statement(
            tx,
            "UPDATE _fexu_media_uploads SET state = state
             WHERE owner = ? AND file_id = ? AND state IN (0, 1, 3)",
            "UPDATE _fexu_media_uploads SET state = state
             WHERE owner = $1 AND file_id = $2 AND state IN (0, 1, 3)",
            media,
        )?;
        change_one(tx, query).await
    }

    async fn has_fence(db: &impl ConnectionTrait, media: MediaRef) -> Result<bool, StorageError> {
        let query = statement(
            db,
            "SELECT file_id FROM _fexu_media_upload_fences WHERE owner = ? AND file_id = ?",
            "SELECT file_id FROM _fexu_media_upload_fences WHERE owner = $1 AND file_id = $2",
            media,
        )?;
        Ok(db
            .query_one(query)
            .await
            .map_err(|_| StorageError::Failed)?
            .is_some())
    }

    async fn has_local_fence(
        db: &impl ConnectionTrait,
        media: MediaRef,
    ) -> Result<bool, StorageError> {
        let query = statement(
            db,
            "SELECT file_id FROM _fexu_media_local_fences WHERE owner = ? AND file_id = ?",
            "SELECT file_id FROM _fexu_media_local_fences WHERE owner = $1 AND file_id = $2",
            media,
        )?;
        Ok(db
            .query_one(query)
            .await
            .map_err(|_| StorageError::Failed)?
            .is_some())
    }

    /// True once irreversible local discard has been requested.
    pub async fn local_fence_requested(&self, media: MediaRef) -> Result<bool, StorageError> {
        Self::has_local_fence(&self.db, media).await
    }

    /// Discard an unresolved upload using the original trusted local root.
    /// All writers must use the exclusive slot protocol. An existing empty
    /// slot is ambiguous and must never be stolen automatically.
    pub async fn abandon_unresolved_local(
        &self,
        storage: &crate::LocalStorage,
        media: MediaRef,
    ) -> Result<(), StorageError> {
        let tx = self.db.begin().await.map_err(|_| StorageError::Failed)?;
        Self::lock_upload(&tx, media).await?;
        if Self::has_fence(&tx, media).await? {
            return Err(StorageError::Failed);
        }
        let query = statement(
            &tx,
            "INSERT INTO _fexu_media_local_fences
             SELECT owner, file_id FROM _fexu_media_uploads
             WHERE owner = ? AND file_id = ? AND state = 0
             ON CONFLICT (owner, file_id) DO NOTHING",
            "INSERT INTO _fexu_media_local_fences
             SELECT owner, file_id FROM _fexu_media_uploads
             WHERE owner = $1 AND file_id = $2 AND state = 0
             ON CONFLICT (owner, file_id) DO NOTHING",
            media,
        )?;
        tx.execute(query).await.map_err(|_| StorageError::Failed)?;
        if !Self::has_local_fence(&tx, media).await? {
            return Err(StorageError::Failed);
        }
        tx.commit().await.map_err(|_| StorageError::Failed)?;
        storage.fence_local_writer(media)?;
        self.confirm_local_discard(media).await
    }

    /// Offline operator recovery of a stale local writer slot after every
    /// writer and competing recovery operator has been stopped and settled.
    /// Requires a previously committed local discard request. Never use age
    /// or a missing process ID as proof. Keep the same trusted storage root.
    /// The permanent marker is preserved, including on retry.
    pub async fn recover_settled_local_discard(
        &self,
        storage: &crate::LocalStorage,
        media: MediaRef,
    ) -> Result<(), StorageError> {
        if !self.local_fence_requested(media).await?
            || self.upload_fence_requested(media).await?
            || !matches!(
                self.upload_state(media).await?,
                Some(UploadState::Uploading | UploadState::Abandoned)
            )
        {
            return Err(StorageError::Failed);
        }
        storage.recover_settled_slot(media)?;
        self.confirm_local_discard(media).await
    }

    async fn confirm_local_discard(&self, media: MediaRef) -> Result<(), StorageError> {
        let tx = self.db.begin().await.map_err(|_| StorageError::Failed)?;
        Self::lock_upload(&tx, media).await?;
        if !Self::has_local_fence(&tx, media).await? || Self::has_fence(&tx, media).await? {
            return Err(StorageError::Failed);
        }
        let query = statement(
            &tx,
            "UPDATE _fexu_media_uploads SET state = 3
             WHERE owner = ? AND file_id = ? AND state IN (0, 3)",
            "UPDATE _fexu_media_uploads SET state = 3
             WHERE owner = $1 AND file_id = $2 AND state IN (0, 3)",
            media,
        )?;
        change_one(&tx, query).await?;
        tx.commit().await.map_err(|_| StorageError::Failed)
    }

    /// True once irreversible S3 discard has been requested, even if the
    /// remote marker has not yet been confirmed.
    pub async fn upload_fence_requested(&self, media: MediaRef) -> Result<bool, StorageError> {
        Self::has_fence(&self.db, media).await
    }

    #[cfg(any(feature = "s3-storage", test))]
    async fn request_upload_fence(&self, media: MediaRef) -> Result<(), StorageError> {
        let tx = self.db.begin().await.map_err(|_| StorageError::Failed)?;
        Self::lock_upload(&tx, media).await?;
        if Self::has_local_fence(&tx, media).await? {
            return Err(StorageError::Failed);
        }
        let query = statement(
            &tx,
            "INSERT INTO _fexu_media_upload_fences
             SELECT owner, file_id FROM _fexu_media_uploads
             WHERE owner = ? AND file_id = ? AND state = 0
             ON CONFLICT (owner, file_id) DO NOTHING",
            "INSERT INTO _fexu_media_upload_fences
             SELECT owner, file_id FROM _fexu_media_uploads
             WHERE owner = $1 AND file_id = $2 AND state = 0
             ON CONFLICT (owner, file_id) DO NOTHING",
            media,
        )?;
        tx.execute(query).await.map_err(|_| StorageError::Failed)?;
        if !Self::has_fence(&tx, media).await? {
            return Err(StorageError::Failed);
        }
        tx.commit().await.map_err(|_| StorageError::Failed)
    }

    /// Irreversibly discard an unresolved upload in the original S3 namespace.
    /// Only authorized operators may call this. The durable request blocks
    /// staging before remote I/O; errors leave a retryable protected request.
    /// The permanent marker must never be deleted by bucket lifecycle rules.
    #[cfg(feature = "s3-storage")]
    pub async fn abandon_unresolved_s3(
        &self,
        storage: &crate::S3Storage,
        media: MediaRef,
    ) -> Result<(), StorageError> {
        self.request_upload_fence(media).await?;
        storage.fence_upload(media).await?;
        let tx = self.db.begin().await.map_err(|_| StorageError::Failed)?;
        Self::lock_upload(&tx, media).await?;
        let query = statement(
            &tx,
            "UPDATE _fexu_media_uploads SET state = 3
             WHERE owner = ? AND file_id = ? AND state IN (0, 3)",
            "UPDATE _fexu_media_uploads SET state = 3
             WHERE owner = $1 AND file_id = $2 AND state IN (0, 3)",
            media,
        )?;
        change_one(&tx, query).await?;
        tx.commit().await.map_err(|_| StorageError::Failed)
    }

    async fn finish_upload(&self, media: MediaRef) -> Result<(), StorageError> {
        let tx = self.db.begin().await.map_err(|_| StorageError::Failed)?;
        Self::lock_upload(&tx, media).await?;
        if Self::has_fence(&tx, media).await? || Self::has_local_fence(&tx, media).await? {
            return Err(StorageError::Failed);
        }
        let query = statement(
            &tx,
            "UPDATE _fexu_media_uploads SET state = 1
             WHERE owner = ? AND file_id = ? AND state = 0",
            "UPDATE _fexu_media_uploads SET state = 1
             WHERE owner = $1 AND file_id = $2 AND state = 0",
            media,
        )?;
        change_one(&tx, query).await?;
        tx.commit().await.map_err(|_| StorageError::Failed)
    }

    /// Observe upload state without retiring the ID or authorizing deletion.
    /// Missing records include legacy enrollments.
    pub async fn upload_state(&self, media: MediaRef) -> Result<Option<UploadState>, StorageError> {
        let query = statement(
            &self.db,
            "SELECT state FROM _fexu_media_uploads WHERE owner = ? AND file_id = ?",
            "SELECT state FROM _fexu_media_uploads WHERE owner = $1 AND file_id = $2",
            media,
        )?;
        let row = self
            .db
            .query_one(query)
            .await
            .map_err(|_| StorageError::Failed)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let state: i32 = row.try_get("", "state").map_err(|_| StorageError::Failed)?;
        match state {
            0 => Ok(Some(UploadState::Uploading)),
            1 => Ok(Some(UploadState::Staged)),
            2 => Ok(Some(UploadState::Committed)),
            3 => Ok(Some(UploadState::Abandoned)),
            _ => Err(StorageError::Failed),
        }
    }

    /// Inspect a currently unresolved upload without changing its state.
    /// A valid readable object is evidence of presence, not of writer
    /// settlement. Missing, expired and failed reads never authorize deletion.
    /// Use the same authorized storage namespace as the original upload.
    pub async fn inspect_unresolved<S: crate::Storage + ?Sized>(
        &self,
        storage: &S,
        media: MediaRef,
    ) -> Result<UploadObservation, StorageError> {
        if self.upload_state(media).await? != Some(UploadState::Uploading) {
            return Ok(UploadObservation::NotUploading);
        }
        let observed = match storage.read(media.owner, media.id).await {
            Ok((file, bytes)) => {
                if file.owner != media.owner
                    || file.id != media.id
                    || file.expires_at.is_none()
                    || file.size != bytes.len() as u64
                    || file.sha256.is_empty()
                    || !crate::local::bytes_match(&file, &bytes)
                {
                    return Err(StorageError::Failed);
                }
                UploadObservation::Present(file)
            }
            Err(StorageError::NotFound) => UploadObservation::Missing,
            Err(StorageError::Expired) => UploadObservation::Expired,
            Err(error) => return Err(error),
        };
        if self.upload_state(media).await? != Some(UploadState::Uploading) {
            return Ok(UploadObservation::NotUploading);
        }
        Ok(observed)
    }

    /// Recover a validated upload after an operator has established that
    /// all original writes have settled. This never creates a model reference.
    /// Missing, mismatched or unjournaled data remains unresolved; a concurrent state change
    /// fails closed. The caller must still authorize any subsequent model save.
    pub async fn recover_settled_upload<S: crate::Storage + ?Sized>(
        &self,
        storage: &S,
        media: MediaRef,
    ) -> Result<crate::StoredFile, StorageError> {
        let UploadObservation::Present(file) = self.inspect_unresolved(storage, media).await?
        else {
            return Err(StorageError::Failed);
        };
        let intent = self
            .upload_intent(media)
            .await?
            .ok_or(StorageError::Failed)?;
        if !intent.matches(&file) {
            return Err(StorageError::Failed);
        }
        self.finish_upload(media).await?;
        Ok(file)
    }

    /// Confirm an upload is permanently abandoned. Only call after proving
    /// that all storage writes have settled, including ambiguous responses.
    /// This does not delete bytes or authorize cleanup of referenced files.
    pub async fn abandon_upload(&self, media: MediaRef) -> Result<(), StorageError> {
        let tx = self.db.begin().await.map_err(|_| StorageError::Failed)?;
        Self::lock_upload(&tx, media).await?;
        if Self::has_fence(&tx, media).await? || Self::has_local_fence(&tx, media).await? {
            return Err(StorageError::Failed);
        }
        let query = statement(
            &tx,
            "UPDATE _fexu_media_uploads SET state = 3
             WHERE owner = ? AND file_id = ? AND state IN (0, 1, 3)",
            "UPDATE _fexu_media_uploads SET state = 3
             WHERE owner = $1 AND file_id = $2 AND state IN (0, 1, 3)",
            media,
        )?;
        change_one(&tx, query).await?;
        tx.commit().await.map_err(|_| StorageError::Failed)
    }

    /// List a bounded, deterministic page of enrolled zero-reference IDs.
    /// This is discovery only: a concurrent attach can invalidate the result.
    /// Use a durable retirement check before any physical removal.
    pub async fn orphan_candidates(
        &self,
        after: Option<MediaRef>,
        limit: u32,
    ) -> Result<Vec<MediaRef>, StorageError> {
        if !(1..=1000).contains(&limit) {
            return Err(StorageError::Failed);
        }
        let backend = self.db.get_database_backend();
        let (sql, values) = match (backend, after) {
            (DatabaseBackend::Sqlite, None) => (
                "SELECT owner, file_id FROM _fexu_media_references
                 WHERE refs = 0 ORDER BY owner, file_id LIMIT ?",
                vec![Value::from(i64::from(limit))],
            ),
            (DatabaseBackend::Postgres, None) => (
                "SELECT owner, file_id FROM _fexu_media_references
                 WHERE refs = 0 ORDER BY owner, file_id LIMIT $1",
                vec![Value::from(i64::from(limit))],
            ),
            (DatabaseBackend::Sqlite, Some(cursor)) => (
                "SELECT owner, file_id FROM _fexu_media_references
                 WHERE refs = 0 AND (owner, file_id) > (?, ?)
                 ORDER BY owner, file_id LIMIT ?",
                vec![
                    Value::from(cursor.owner.to_string()),
                    Value::from(cursor.id.to_string()),
                    Value::from(i64::from(limit)),
                ],
            ),
            (DatabaseBackend::Postgres, Some(cursor)) => (
                "SELECT owner, file_id FROM _fexu_media_references
                 WHERE refs = 0 AND (owner, file_id) > ($1, $2)
                 ORDER BY owner, file_id LIMIT $3",
                vec![
                    Value::from(cursor.owner.to_string()),
                    Value::from(cursor.id.to_string()),
                    Value::from(i64::from(limit)),
                ],
            ),
            _ => return Err(StorageError::Failed),
        };
        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(backend, sql, values))
            .await
            .map_err(|_| StorageError::Failed)?;
        rows.into_iter()
            .map(|row| {
                let owner: String = row.try_get("", "owner").map_err(|_| StorageError::Failed)?;
                let id: String = row
                    .try_get("", "file_id")
                    .map_err(|_| StorageError::Failed)?;
                let owner = uuid::Uuid::parse_str(&owner).map_err(|_| StorageError::Failed)?;
                let id = uuid::Uuid::parse_str(&id).map_err(|_| StorageError::Failed)?;
                Ok(MediaRef { owner, id })
            })
            .collect()
    }

    /// Discover only explicitly abandoned, tracked zero-reference IDs.
    /// Active, staged, committed and legacy IDs are excluded. Retired IDs
    /// without a recorded result remain visible for retry. This is not permission to delete.
    pub async fn abandoned_candidates(
        &self,
        after: Option<MediaRef>,
        limit: u32,
    ) -> Result<Vec<MediaRef>, StorageError> {
        if !(1..=1000).contains(&limit) {
            return Err(StorageError::Failed);
        }
        let backend = self.db.get_database_backend();
        let (sql, values) = match (backend, after) {
            (DatabaseBackend::Sqlite, None) => (
                "SELECT r.owner, r.file_id FROM _fexu_media_references r
                 JOIN _fexu_media_uploads u ON u.owner = r.owner AND u.file_id = r.file_id
                 WHERE r.refs = 0 AND u.state = 3
                 AND NOT EXISTS (SELECT 1 FROM _fexu_media_cleanup_receipts c
                     WHERE c.owner = r.owner AND c.file_id = r.file_id) ORDER BY r.owner, r.file_id LIMIT ?",
                vec![Value::from(i64::from(limit))],
            ),
            (DatabaseBackend::Postgres, None) => (
                "SELECT r.owner, r.file_id FROM _fexu_media_references r
                 JOIN _fexu_media_uploads u ON u.owner = r.owner AND u.file_id = r.file_id
                 WHERE r.refs = 0 AND u.state = 3
                 AND NOT EXISTS (SELECT 1 FROM _fexu_media_cleanup_receipts c
                     WHERE c.owner = r.owner AND c.file_id = r.file_id) ORDER BY r.owner, r.file_id LIMIT $1",
                vec![Value::from(i64::from(limit))],
            ),
            (DatabaseBackend::Sqlite, Some(cursor)) => (
                "SELECT r.owner, r.file_id FROM _fexu_media_references r
                 JOIN _fexu_media_uploads u ON u.owner = r.owner AND u.file_id = r.file_id
                 WHERE r.refs = 0 AND u.state = 3
                 AND NOT EXISTS (SELECT 1 FROM _fexu_media_cleanup_receipts c
                     WHERE c.owner = r.owner AND c.file_id = r.file_id) AND (r.owner, r.file_id) > (?, ?)
                 ORDER BY r.owner, r.file_id LIMIT ?",
                vec![
                    Value::from(cursor.owner.to_string()),
                    Value::from(cursor.id.to_string()),
                    Value::from(i64::from(limit)),
                ],
            ),
            (DatabaseBackend::Postgres, Some(cursor)) => (
                "SELECT r.owner, r.file_id FROM _fexu_media_references r
                 JOIN _fexu_media_uploads u ON u.owner = r.owner AND u.file_id = r.file_id
                 WHERE r.refs = 0 AND u.state = 3
                 AND NOT EXISTS (SELECT 1 FROM _fexu_media_cleanup_receipts c
                     WHERE c.owner = r.owner AND c.file_id = r.file_id) AND (r.owner, r.file_id) > ($1, $2)
                 ORDER BY r.owner, r.file_id LIMIT $3",
                vec![
                    Value::from(cursor.owner.to_string()),
                    Value::from(cursor.id.to_string()),
                    Value::from(i64::from(limit)),
                ],
            ),
            _ => return Err(StorageError::Failed),
        };
        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(backend, sql, values))
            .await
            .map_err(|_| StorageError::Failed)?;
        rows.into_iter()
            .map(|row| {
                let owner: String = row.try_get("", "owner").map_err(|_| StorageError::Failed)?;
                let id: String = row
                    .try_get("", "file_id")
                    .map_err(|_| StorageError::Failed)?;
                let owner = uuid::Uuid::parse_str(&owner).map_err(|_| StorageError::Failed)?;
                let id = uuid::Uuid::parse_str(&id).map_err(|_| StorageError::Failed)?;
                Ok(MediaRef { owner, id })
            })
            .collect()
    }

    async fn local_quota_candidates(
        &self,
        after: Option<MediaRef>,
        limit: u32,
    ) -> Result<Vec<MediaRef>, StorageError> {
        if !(1..=1000).contains(&limit) {
            return Err(StorageError::Failed);
        }
        let backend = self.db.get_database_backend();
        let (sql, values) = match (backend, after) {
            (DatabaseBackend::Sqlite, None) => (
                "SELECT r.owner, r.file_id FROM _fexu_media_references r
                 JOIN _fexu_media_uploads u ON u.owner = r.owner AND u.file_id = r.file_id
                 WHERE r.refs = 0 AND u.state = 3
                 AND EXISTS (SELECT 1 FROM _fexu_media_quota_charges q
                     WHERE q.owner = r.owner AND q.file_id = r.file_id AND q.released = 0)
                 AND NOT EXISTS (SELECT 1 FROM _fexu_media_upload_fences f
                     WHERE f.owner = r.owner AND f.file_id = r.file_id) ORDER BY r.owner, r.file_id LIMIT ?",
                vec![Value::from(i64::from(limit))],
            ),
            (DatabaseBackend::Postgres, None) => (
                "SELECT r.owner, r.file_id FROM _fexu_media_references r
                 JOIN _fexu_media_uploads u ON u.owner = r.owner AND u.file_id = r.file_id
                 WHERE r.refs = 0 AND u.state = 3
                 AND EXISTS (SELECT 1 FROM _fexu_media_quota_charges q
                     WHERE q.owner = r.owner AND q.file_id = r.file_id AND q.released = 0)
                 AND NOT EXISTS (SELECT 1 FROM _fexu_media_upload_fences f
                     WHERE f.owner = r.owner AND f.file_id = r.file_id) ORDER BY r.owner, r.file_id LIMIT $1",
                vec![Value::from(i64::from(limit))],
            ),
            (DatabaseBackend::Sqlite, Some(cursor)) => (
                "SELECT r.owner, r.file_id FROM _fexu_media_references r
                 JOIN _fexu_media_uploads u ON u.owner = r.owner AND u.file_id = r.file_id
                 WHERE r.refs = 0 AND u.state = 3
                 AND EXISTS (SELECT 1 FROM _fexu_media_quota_charges q
                     WHERE q.owner = r.owner AND q.file_id = r.file_id AND q.released = 0)
                 AND NOT EXISTS (SELECT 1 FROM _fexu_media_upload_fences f
                     WHERE f.owner = r.owner AND f.file_id = r.file_id) AND (r.owner, r.file_id) > (?, ?)
                 ORDER BY r.owner, r.file_id LIMIT ?",
                vec![
                    Value::from(cursor.owner.to_string()),
                    Value::from(cursor.id.to_string()),
                    Value::from(i64::from(limit)),
                ],
            ),
            (DatabaseBackend::Postgres, Some(cursor)) => (
                "SELECT r.owner, r.file_id FROM _fexu_media_references r
                 JOIN _fexu_media_uploads u ON u.owner = r.owner AND u.file_id = r.file_id
                 WHERE r.refs = 0 AND u.state = 3
                 AND EXISTS (SELECT 1 FROM _fexu_media_quota_charges q
                     WHERE q.owner = r.owner AND q.file_id = r.file_id AND q.released = 0)
                 AND NOT EXISTS (SELECT 1 FROM _fexu_media_upload_fences f
                     WHERE f.owner = r.owner AND f.file_id = r.file_id) AND (r.owner, r.file_id) > ($1, $2)
                 ORDER BY r.owner, r.file_id LIMIT $3",
                vec![
                    Value::from(cursor.owner.to_string()),
                    Value::from(cursor.id.to_string()),
                    Value::from(i64::from(limit)),
                ],
            ),
            _ => return Err(StorageError::Failed),
        };
        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(backend, sql, values))
            .await
            .map_err(|_| StorageError::Failed)?;
        rows.into_iter()
            .map(|row| {
                let owner: String = row.try_get("", "owner").map_err(|_| StorageError::Failed)?;
                let id: String = row
                    .try_get("", "file_id")
                    .map_err(|_| StorageError::Failed)?;
                let owner = uuid::Uuid::parse_str(&owner).map_err(|_| StorageError::Failed)?;
                let id = uuid::Uuid::parse_str(&id).map_err(|_| StorageError::Failed)?;
                Ok(MediaRef { owner, id })
            })
            .collect()
    }

    /// Discover unresolved uploads, including writes still in progress.
    /// Results are observations only; neither age nor absence authorizes cleanup.
    pub async fn unresolved_uploads(
        &self,
        after: Option<MediaRef>,
        limit: u32,
    ) -> Result<Vec<MediaRef>, StorageError> {
        if !(1..=1000).contains(&limit) {
            return Err(StorageError::Failed);
        }
        let backend = self.db.get_database_backend();
        let (sql, values) = match (backend, after) {
            (DatabaseBackend::Sqlite, None) => (
                "SELECT r.owner, r.file_id FROM _fexu_media_references r
                 JOIN _fexu_media_uploads u ON u.owner = r.owner AND u.file_id = r.file_id
                 WHERE r.refs = 0 AND r.retired = 0 AND u.state = 0 ORDER BY r.owner, r.file_id LIMIT ?",
                vec![Value::from(i64::from(limit))],
            ),
            (DatabaseBackend::Postgres, None) => (
                "SELECT r.owner, r.file_id FROM _fexu_media_references r
                 JOIN _fexu_media_uploads u ON u.owner = r.owner AND u.file_id = r.file_id
                 WHERE r.refs = 0 AND r.retired = 0 AND u.state = 0 ORDER BY r.owner, r.file_id LIMIT $1",
                vec![Value::from(i64::from(limit))],
            ),
            (DatabaseBackend::Sqlite, Some(cursor)) => (
                "SELECT r.owner, r.file_id FROM _fexu_media_references r
                 JOIN _fexu_media_uploads u ON u.owner = r.owner AND u.file_id = r.file_id
                 WHERE r.refs = 0 AND r.retired = 0 AND u.state = 0 AND (r.owner, r.file_id) > (?, ?)
                 ORDER BY r.owner, r.file_id LIMIT ?",
                vec![
                    Value::from(cursor.owner.to_string()),
                    Value::from(cursor.id.to_string()),
                    Value::from(i64::from(limit)),
                ],
            ),
            (DatabaseBackend::Postgres, Some(cursor)) => (
                "SELECT r.owner, r.file_id FROM _fexu_media_references r
                 JOIN _fexu_media_uploads u ON u.owner = r.owner AND u.file_id = r.file_id
                 WHERE r.refs = 0 AND r.retired = 0 AND u.state = 0 AND (r.owner, r.file_id) > ($1, $2)
                 ORDER BY r.owner, r.file_id LIMIT $3",
                vec![
                    Value::from(cursor.owner.to_string()),
                    Value::from(cursor.id.to_string()),
                    Value::from(i64::from(limit)),
                ],
            ),
            _ => return Err(StorageError::Failed),
        };
        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(backend, sql, values))
            .await
            .map_err(|_| StorageError::Failed)?;
        rows.into_iter()
            .map(|row| {
                let owner: String = row.try_get("", "owner").map_err(|_| StorageError::Failed)?;
                let id: String = row
                    .try_get("", "file_id")
                    .map_err(|_| StorageError::Failed)?;
                let owner = uuid::Uuid::parse_str(&owner).map_err(|_| StorageError::Failed)?;
                let id = uuid::Uuid::parse_str(&id).map_err(|_| StorageError::Failed)?;
                Ok(MediaRef { owner, id })
            })
            .collect()
    }

    /// Process one bounded page of explicitly abandoned uploads.
    /// No implicit abandonment, timeout or deletion of legacy IDs occurs.
    /// Each selected ID is checked again by the durable retirement gate.
    /// A failed item is returned without aborting the rest of the page.
    /// Backend results are durably recorded before an item reports success.
    /// Resume with the last result's ID; revisit earlier pages on later runs.
    pub async fn cleanup_abandoned_page<S: crate::Storage + ?Sized>(
        &self,
        storage: &S,
        after: Option<MediaRef>,
        limit: u32,
    ) -> Result<Vec<AbandonedCleanupItem>, StorageError> {
        let candidates = self.abandoned_candidates(after, limit).await?;
        let mut results = Vec::with_capacity(candidates.len());
        for media in candidates {
            let outcome = self.cleanup_with_receipt(storage, media).await;
            results.push(AbandonedCleanupItem { media, outcome });
        }
        Ok(results)
    }

    /// Run a bounded cleanup sweep for a single trusted storage namespace.
    /// Intended for an explicitly configured scheduler or worker handler.
    /// Never abandons uploads, scans raw storage, or deletes committed IDs.
    /// A page-query failure preserves progress and earlier item results.
    /// At most 16 pages and 1,000 total candidates may be attempted per call.
    pub async fn cleanup_abandoned_batch<S: crate::Storage + ?Sized>(
        &self,
        storage: &S,
        after: Option<MediaRef>,
        page_size: u32,
        max_pages: u32,
    ) -> Result<CleanupBatch, StorageError> {
        if !(1..=16).contains(&max_pages)
            || !(1..=1000).contains(&page_size)
            || page_size * max_pages > 1000
        {
            return Err(StorageError::Failed);
        }
        let mut batch = CleanupBatch {
            items: Vec::new(),
            next_after: after,
            reached_end: false,
            scan_error: None,
        };
        for _ in 0..max_pages {
            let page = match self
                .cleanup_abandoned_page(storage, batch.next_after, page_size)
                .await
            {
                Ok(page) => page,
                Err(error) => {
                    batch.scan_error = Some(error);
                    break;
                }
            };
            let short = page.len() < page_size as usize;
            if let Some(last) = page.last() {
                batch.next_after = Some(last.media);
            }
            batch.items.extend(page);
            if short {
                batch.reached_end = true;
                batch.next_after = None;
                break;
            }
        }
        Ok(batch)
    }

    /// Sweep only unreleased, journaled, explicitly abandoned reservations.
    /// The entire ledger must belong to the supplied original local namespace.
    /// Never use this with a mixed-backend or mixed-root ledger.
    /// Existing receipts do not exclude candidates; a crash before release
    /// remains retryable. At most 16 pages and 1,000 IDs are attempted.
    /// Restart from None after reaching the end to revisit failed items.
    pub async fn cleanup_local_quota_batch(
        &self,
        storage: &crate::LocalStorage,
        after: Option<MediaRef>,
        page_size: u32,
        max_pages: u32,
    ) -> Result<LocalQuotaCleanupBatch, StorageError> {
        if !(1..=16).contains(&max_pages)
            || !(1..=1000).contains(&page_size)
            || page_size * max_pages > 1000
        {
            return Err(StorageError::Failed);
        }
        let mut batch = LocalQuotaCleanupBatch {
            items: Vec::new(),
            next_after: after,
            reached_end: false,
            scan_error: None,
        };
        for _ in 0..max_pages {
            let candidates = match self
                .local_quota_candidates(batch.next_after, page_size)
                .await
            {
                Ok(candidates) => candidates,
                Err(error) => {
                    batch.scan_error = Some(error);
                    break;
                }
            };
            let short = candidates.len() < page_size as usize;
            for media in candidates {
                let outcome = self.cleanup_and_release_local_quota(storage, media).await;
                batch.items.push(LocalQuotaCleanupItem { media, outcome });
                batch.next_after = Some(media);
            }
            if short {
                batch.reached_end = true;
                batch.next_after = None;
                break;
            }
        }
        Ok(batch)
    }

    /// Read a previously recorded abandoned-page backend outcome.
    pub async fn cleanup_receipt(
        &self,
        media: MediaRef,
    ) -> Result<Option<CleanupReceipt>, StorageError> {
        let query = statement(
            &self.db,
            "SELECT outcome FROM _fexu_media_cleanup_receipts WHERE owner = ? AND file_id = ?",
            "SELECT outcome FROM _fexu_media_cleanup_receipts WHERE owner = $1 AND file_id = $2",
            media,
        )?;
        let row = self
            .db
            .query_one(query)
            .await
            .map_err(|_| StorageError::Failed)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let outcome: i32 = row
            .try_get("", "outcome")
            .map_err(|_| StorageError::Failed)?;
        match outcome {
            1 => Ok(Some(CleanupReceipt::Removed)),
            2 => Ok(Some(CleanupReceipt::Missing)),
            _ => Err(StorageError::Failed),
        }
    }

    async fn cleanup_with_receipt<S: crate::Storage + ?Sized>(
        &self,
        storage: &S,
        media: MediaRef,
    ) -> Result<AbandonedCleanup, StorageError> {
        if let Some(receipt) = self.cleanup_receipt(media).await? {
            return Ok(match receipt {
                CleanupReceipt::Removed => AbandonedCleanup::Removed,
                CleanupReceipt::Missing => AbandonedCleanup::Missing,
            });
        }
        let outcome = self.cleanup_abandoned(storage, media).await?;
        self.record_cleanup_receipt(media, outcome).await
    }

    async fn record_cleanup_receipt(
        &self,
        media: MediaRef,
        outcome: AbandonedCleanup,
    ) -> Result<AbandonedCleanup, StorageError> {
        let (sqlite, postgres) = match outcome {
            AbandonedCleanup::Removed => (
                "INSERT INTO _fexu_media_cleanup_receipts VALUES (?, ?, 1)
                 ON CONFLICT (owner, file_id) DO NOTHING",
                "INSERT INTO _fexu_media_cleanup_receipts VALUES ($1, $2, 1)
                 ON CONFLICT (owner, file_id) DO NOTHING",
            ),
            AbandonedCleanup::Missing => (
                "INSERT INTO _fexu_media_cleanup_receipts VALUES (?, ?, 2)
                 ON CONFLICT (owner, file_id) DO NOTHING",
                "INSERT INTO _fexu_media_cleanup_receipts VALUES ($1, $2, 2)
                 ON CONFLICT (owner, file_id) DO NOTHING",
            ),
            _ => return Ok(outcome),
        };
        let query = statement(&self.db, sqlite, postgres, media)?;
        self.db
            .execute(query)
            .await
            .map_err(|_| StorageError::Failed)?;
        // Concurrent workers may have observed different backend outcomes.
        // Report the first durable receipt rather than overwrite it.
        match self.cleanup_receipt(media).await? {
            Some(CleanupReceipt::Removed) => Ok(AbandonedCleanup::Removed),
            Some(CleanupReceipt::Missing) => Ok(AbandonedCleanup::Missing),
            None => Err(StorageError::Failed),
        }
    }

    /// Reconcile and release a permanently abandoned local reservation.
    /// An authorized caller must select the original trusted local root.
    /// All writers must cooperate with the exclusive slot protocol; exclude
    /// raw mutation, owner purge and legacy cleanup. Never steal a busy slot.
    /// The permanent marker survives cleanup. Existing receipts do not skip
    /// residue reconciliation. Returns true only for a newly released charge.
    /// No automatic abandonment or remote quota release is performed.
    pub async fn cleanup_and_release_local_quota(
        &self,
        storage: &crate::LocalStorage,
        media: MediaRef,
    ) -> Result<bool, StorageError> {
        if self.upload_state(media).await? != Some(UploadState::Abandoned)
            || self.upload_fence_requested(media).await?
        {
            return Ok(false);
        }
        match self.check(media).await? {
            ReferenceDecision::Unknown | ReferenceDecision::Referenced => return Ok(false),
            ReferenceDecision::Unreferenced(()) => {}
        }
        storage.fence_local_writer(media)?;
        let outcome = if storage.cleanup_retired(media)? {
            AbandonedCleanup::Removed
        } else {
            AbandonedCleanup::Missing
        };
        self.record_cleanup_receipt(media, outcome).await?;
        self.release_settled_quota(media).await
    }

    /// Permanently fence an explicitly abandoned, zero-reference S3 ID.
    /// Use only the original authorized bucket. Settle destructive operations
    /// before calling; ordinary deletes and expiry cleanup must not overlap.
    /// The durable request survives failures. Retry conditional-write errors.
    /// This does not release quota or verify historical object versions.
    #[cfg(feature = "s3-storage")]
    pub async fn fence_abandoned_s3(
        &self,
        storage: &crate::S3Storage,
        media: MediaRef,
    ) -> Result<bool, StorageError> {
        if self.upload_state(media).await? != Some(UploadState::Abandoned)
            || self.local_fence_requested(media).await?
        {
            return Ok(false);
        }
        match self.check(media).await? {
            ReferenceDecision::Unknown | ReferenceDecision::Referenced => return Ok(false),
            ReferenceDecision::Unreferenced(()) => {}
        }
        let tx = self.db.begin().await.map_err(|_| StorageError::Failed)?;
        Self::lock_upload(&tx, media).await?;
        if Self::has_local_fence(&tx, media).await? {
            return Err(StorageError::Failed);
        }
        let query = statement(
            &tx,
            "INSERT INTO _fexu_media_upload_fences
             SELECT u.owner, u.file_id FROM _fexu_media_uploads u
             JOIN _fexu_media_references r ON r.owner = u.owner AND r.file_id = u.file_id
             WHERE u.owner = ? AND u.file_id = ? AND u.state = 3
             AND r.refs = 0 AND r.retired = 1
             ON CONFLICT (owner, file_id) DO NOTHING",
            "INSERT INTO _fexu_media_upload_fences
             SELECT u.owner, u.file_id FROM _fexu_media_uploads u
             JOIN _fexu_media_references r ON r.owner = u.owner AND r.file_id = u.file_id
             WHERE u.owner = $1 AND u.file_id = $2 AND u.state = 3
             AND r.refs = 0 AND r.retired = 1
             ON CONFLICT (owner, file_id) DO NOTHING",
            media,
        )?;
        tx.execute(query).await.map_err(|_| StorageError::Failed)?;
        if !Self::has_fence(&tx, media).await? {
            return Err(StorageError::Failed);
        }
        tx.commit().await.map_err(|_| StorageError::Failed)?;
        storage.fence_upload(media).await?;
        storage.verify_upload_fence(media).await?;
        self.record_cleanup_receipt(media, AbandonedCleanup::Missing)
            .await?;
        Ok(true)
    }

    /// Release a previously fenced S3 reservation after operator settlement.
    /// The caller must verify the original bucket, settle all destructive
    /// operations and establish that no relevant historical object versions
    /// remain. Keep the permanent marker and prohibit raw mutation forever.
    /// This checks the current marker, not bucket version history or backups.
    /// Missing or unexpected markers fail closed, even with a prior receipt.
    /// Only a newly released journaled charge returns true.
    #[cfg(feature = "s3-storage")]
    pub async fn release_fenced_s3_quota(
        &self,
        storage: &crate::S3Storage,
        media: MediaRef,
    ) -> Result<bool, StorageError> {
        if self.upload_state(media).await? != Some(UploadState::Abandoned)
            || !self.upload_fence_requested(media).await?
            || self.local_fence_requested(media).await?
        {
            return Ok(false);
        }
        match self.check(media).await? {
            ReferenceDecision::Unknown | ReferenceDecision::Referenced => return Ok(false),
            ReferenceDecision::Unreferenced(()) => {}
        }
        storage.verify_upload_fence(media).await?;
        self.record_cleanup_receipt(media, AbandonedCleanup::Missing)
            .await?;
        self.release_settled_quota(media).await
    }

    /// Reconcile the four known local paths of an explicitly abandoned ID.
    /// Unlike ordinary cleanup, this also checks IDs with existing receipts.
    /// The caller must establish local writer settlement and use the original
    /// trusted storage root, excluding concurrent raw filesystem operations.
    pub async fn cleanup_local_residue(
        &self,
        storage: &crate::LocalStorage,
        media: MediaRef,
    ) -> Result<AbandonedCleanup, StorageError> {
        if self.upload_state(media).await? != Some(UploadState::Abandoned)
            || self.upload_fence_requested(media).await?
        {
            return Ok(AbandonedCleanup::Unknown);
        }
        match self.check(media).await? {
            ReferenceDecision::Unknown => Ok(AbandonedCleanup::Unknown),
            ReferenceDecision::Referenced => Ok(AbandonedCleanup::Referenced),
            ReferenceDecision::Unreferenced(()) => {
                if storage.cleanup_retired(media)? {
                    Ok(AbandonedCleanup::Removed)
                } else {
                    Ok(AbandonedCleanup::Missing)
                }
            }
        }
    }

    /// Retire an enrolled zero-reference ID and remove its bytes.
    ///
    /// Call only for candidates that are permanently abandoned, including
    /// retained orphans. Every writer must use the ledger and never reuse IDs.
    /// Unknown or referenced IDs are never deleted. The retirement survives
    /// failed storage requests and can be retried. Missing bytes are reported
    /// separately from confirmed removal. No raw bucket scan is performed.
    pub async fn cleanup_abandoned<S: crate::Storage + ?Sized>(
        &self,
        storage: &S,
        media: MediaRef,
    ) -> Result<AbandonedCleanup, StorageError> {
        match self.check(media).await? {
            ReferenceDecision::Unknown => Ok(AbandonedCleanup::Unknown),
            ReferenceDecision::Referenced => Ok(AbandonedCleanup::Referenced),
            ReferenceDecision::Unreferenced(()) => {
                match storage.delete(media.owner, media.id).await {
                    Ok(()) => Ok(AbandonedCleanup::Removed),
                    Err(StorageError::NotFound) => Ok(AbandonedCleanup::Missing),
                    Err(error) => Err(error),
                }
            }
        }
    }

    /// Enroll a fresh ID within the same transaction as its first model reference.
    pub async fn register_tx(
        tx: &DatabaseTransaction,
        media: MediaRef,
    ) -> Result<(), StorageError> {
        register_on(tx, media).await
    }

    /// Claim an enrolled, previously unreferenced ID for its first model save.
    /// A retired, unknown or already attached ID cannot be claimed.
    pub(crate) async fn claim_reserved(
        tx: &DatabaseTransaction,
        media: MediaRef,
    ) -> Result<(), StorageError> {
        let query = statement(
            tx,
            "UPDATE _fexu_media_references SET refs = 1
             WHERE owner = ? AND file_id = ? AND retired = 0 AND refs = 0",
            "UPDATE _fexu_media_references SET refs = 1
             WHERE owner = $1 AND file_id = $2 AND retired = 0 AND refs = 0",
            media,
        )?;
        change_one(tx, query).await?;
        let query = statement(
            tx,
            "UPDATE _fexu_media_uploads SET state = 2
             WHERE owner = ? AND file_id = ? AND state = 1",
            "UPDATE _fexu_media_uploads SET state = 2
             WHERE owner = $1 AND file_id = $2 AND state = 1",
            media,
        )?;
        change_one(tx, query).await
    }

    /// Call in the same transaction as the corresponding model reference
    /// insertion. Roll back the whole transaction on any error.
    pub async fn attach(tx: &DatabaseTransaction, media: MediaRef) -> Result<(), StorageError> {
        let query = statement(
            tx,
            "UPDATE _fexu_media_references SET refs = refs + 1
             WHERE owner = ? AND file_id = ? AND retired = 0 AND refs < 9223372036854775807
             AND NOT EXISTS (SELECT 1 FROM _fexu_media_uploads u
                 WHERE u.owner = _fexu_media_references.owner
                 AND u.file_id = _fexu_media_references.file_id AND u.state <> 2)",
            "UPDATE _fexu_media_references SET refs = refs + 1
             WHERE owner = $1 AND file_id = $2 AND retired = 0 AND refs < 9223372036854775807
             AND NOT EXISTS (SELECT 1 FROM _fexu_media_uploads u
                 WHERE u.owner = _fexu_media_references.owner
                 AND u.file_id = _fexu_media_references.file_id AND u.state <> 2)",
            media,
        )?;
        change_one(tx, query).await
    }

    /// Explicitly admit a previously committed, now unreferenced upload to
    /// abandoned-only cleanup. The caller must authorize permanent deletion
    /// and establish that no external references or storage writes remain.
    /// Use the same transaction as the model removal, or a fresh transaction
    /// after it. Roll back the entire transaction on error.
    /// Returns false for referenced, unknown, staged or legacy IDs.
    /// A successful admission permanently prevents subsequent attachment.
    pub async fn admit_detached_cleanup(
        tx: &DatabaseTransaction,
        media: MediaRef,
    ) -> Result<bool, StorageError> {
        // Lock the reference row first, matching claim_reserved's lock order.
        let query = statement(
            tx,
            "UPDATE _fexu_media_references SET retired = 1
             WHERE owner = ? AND file_id = ? AND refs = 0
             AND EXISTS (SELECT 1 FROM _fexu_media_uploads u
                 WHERE u.owner = _fexu_media_references.owner
                 AND u.file_id = _fexu_media_references.file_id AND u.state = 2)",
            "UPDATE _fexu_media_references SET retired = 1
             WHERE owner = $1 AND file_id = $2 AND refs = 0
             AND EXISTS (SELECT 1 FROM _fexu_media_uploads u
                 WHERE u.owner = _fexu_media_references.owner
                 AND u.file_id = _fexu_media_references.file_id AND u.state = 2)",
            media,
        )?;
        let changed = tx.execute(query).await.map_err(|_| StorageError::Failed)?;
        if changed.rows_affected() == 0 {
            return Ok(false);
        }
        if changed.rows_affected() != 1 {
            return Err(StorageError::Failed);
        }
        let query = statement(
            tx,
            "UPDATE _fexu_media_uploads SET state = 3
             WHERE owner = ? AND file_id = ? AND state = 2",
            "UPDATE _fexu_media_uploads SET state = 3
             WHERE owner = $1 AND file_id = $2 AND state = 2",
            media,
        )?;
        change_one(tx, query).await?;
        Ok(true)
    }

    /// Call in the same transaction as the corresponding model reference
    /// removal. Underflow and unknown or retired IDs are rejected.
    pub async fn detach(tx: &DatabaseTransaction, media: MediaRef) -> Result<(), StorageError> {
        let query = statement(
            tx,
            "UPDATE _fexu_media_references SET refs = refs - 1
             WHERE owner = ? AND file_id = ? AND retired = 0 AND refs > 0",
            "UPDATE _fexu_media_references SET refs = refs - 1
             WHERE owner = $1 AND file_id = $2 AND retired = 0 AND refs > 0",
            media,
        )?;
        change_one(tx, query).await
    }
}

async fn register_on(db: &impl ConnectionTrait, media: MediaRef) -> Result<(), StorageError> {
    let query = statement(
        db,
        "INSERT INTO _fexu_media_references VALUES (?, ?, 0, 0)",
        "INSERT INTO _fexu_media_references VALUES ($1, $2, 0, 0)",
        media,
    )?;
    db.execute(query).await.map_err(|_| StorageError::Failed)?;
    Ok(())
}

async fn change_one(tx: &DatabaseTransaction, query: Statement) -> Result<(), StorageError> {
    let changed = tx.execute(query).await.map_err(|_| StorageError::Failed)?;
    if changed.rows_affected() != 1 {
        return Err(StorageError::Failed);
    }
    Ok(())
}

#[async_trait]
impl ReferenceCheck for ReferenceLedger {
    // The committed retirement row is the durable fence, not a live lock.
    type Guard = ();

    async fn check(&self, media: MediaRef) -> Result<ReferenceDecision<()>, StorageError> {
        let tx = self.db.begin().await.map_err(|_| StorageError::Failed)?;
        // First statement is a write on both backends. It serializes against
        // attach/detach without a SQLite read-to-write lock upgrade.
        let query = statement(
            &tx,
            "UPDATE _fexu_media_references SET retired = 1
             WHERE owner = ? AND file_id = ? AND refs = 0 AND retired = 0
             AND NOT EXISTS (SELECT 1 FROM _fexu_media_uploads u
                 WHERE u.owner = _fexu_media_references.owner
                 AND u.file_id = _fexu_media_references.file_id AND u.state IN (0, 1))",
            "UPDATE _fexu_media_references SET retired = 1
             WHERE owner = $1 AND file_id = $2 AND refs = 0 AND retired = 0
             AND NOT EXISTS (SELECT 1 FROM _fexu_media_uploads u
                 WHERE u.owner = _fexu_media_references.owner
                 AND u.file_id = _fexu_media_references.file_id AND u.state IN (0, 1))",
            media,
        )?;
        tx.execute(query).await.map_err(|_| StorageError::Failed)?;
        let query = statement(
            &tx,
            "SELECT refs, retired FROM _fexu_media_references WHERE owner = ? AND file_id = ?",
            "SELECT refs, retired FROM _fexu_media_references WHERE owner = $1 AND file_id = $2",
            media,
        )?;
        let row = tx
            .query_one(query)
            .await
            .map_err(|_| StorageError::Failed)?;
        let decision = match row {
            None => ReferenceDecision::Unknown,
            Some(row) => {
                let refs: i64 = row.try_get("", "refs").map_err(|_| StorageError::Failed)?;
                let retired: i32 = row
                    .try_get("", "retired")
                    .map_err(|_| StorageError::Failed)?;
                match (refs, retired) {
                    (0, 1) => ReferenceDecision::Unreferenced(()),
                    (1..=i64::MAX, 0) => ReferenceDecision::Referenced,
                    (0, 0) => ReferenceDecision::Unknown,
                    _ => return Err(StorageError::Failed),
                }
            }
        };
        // If commit is ambiguous, no cleanup is authorized by this call.
        tx.commit().await.map_err(|_| StorageError::Failed)?;
        Ok(decision)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    async fn contract(first: &DatabaseConnection, second: &DatabaseConnection) {
        let ledger = ReferenceLedger::new(first.clone());
        let other = ReferenceLedger::new(second.clone());
        ledger.initialize().await.unwrap();
        let media = MediaRef {
            owner: Uuid::new_v4(),
            id: Uuid::new_v4(),
        };
        assert_eq!(
            other.check(media).await.unwrap(),
            ReferenceDecision::Unknown
        );
        ledger.register(media).await.unwrap();
        assert!(ledger.register(media).await.is_err());

        let tx = first.begin().await.unwrap();
        ReferenceLedger::attach(&tx, media).await.unwrap();
        tx.commit().await.unwrap();
        assert_eq!(
            other.check(media).await.unwrap(),
            ReferenceDecision::Referenced
        );

        let tx = first.begin().await.unwrap();
        ReferenceLedger::detach(&tx, media).await.unwrap();
        tx.rollback().await.unwrap();
        assert_eq!(
            other.check(media).await.unwrap(),
            ReferenceDecision::Referenced
        );

        let tx = first.begin().await.unwrap();
        ReferenceLedger::detach(&tx, media).await.unwrap();
        tx.commit().await.unwrap();
        assert_eq!(
            other.check(media).await.unwrap(),
            ReferenceDecision::Unreferenced(())
        );
        let tx = first.begin().await.unwrap();
        assert!(ReferenceLedger::attach(&tx, media).await.is_err());
        tx.rollback().await.unwrap();
        assert!(ledger.register(media).await.is_err());
        assert_eq!(
            ledger.check(media).await.unwrap(),
            ReferenceDecision::Unreferenced(())
        );

        // Independently connected writers race for a new zero-count ID.
        let race = MediaRef {
            owner: media.owner,
            id: Uuid::new_v4(),
        };
        ledger.register(race).await.unwrap();
        let writer = async {
            let tx = first.begin().await.unwrap();
            if ReferenceLedger::attach(&tx, race).await.is_ok() {
                tx.commit().await.unwrap();
                true
            } else {
                tx.rollback().await.unwrap();
                false
            }
        };
        let (attached, decision) = tokio::join!(writer, other.check(race));
        let decision = decision.unwrap();
        if attached {
            assert_eq!(decision, ReferenceDecision::Referenced);
        } else {
            assert_eq!(decision, ReferenceDecision::Unreferenced(()));
        }
    }

    async fn reserved_contract(db: &DatabaseConnection) {
        use crate::{LocalStorage, Storage};

        let root = std::env::temp_dir().join(format!("fexu-reserved-{}", Uuid::new_v4()));
        let storage = LocalStorage::open(&root, 64, &["text/plain"]).unwrap();
        let ledger = ReferenceLedger::new(db.clone());
        ledger.initialize().await.unwrap();
        let owner = Uuid::new_v4();
        let failed = ledger
            .stage_reserved(&storage, owner, "note.txt", &[b'x'; 65], 100)
            .await
            .unwrap_err();
        assert_eq!(failed.error, StorageError::TooLarge);
        assert_eq!(
            ledger.upload_state(failed.media).await.unwrap(),
            Some(UploadState::Uploading)
        );
        assert_eq!(
            ledger
                .cleanup_abandoned(&storage, failed.media)
                .await
                .unwrap(),
            AbandonedCleanup::Unknown
        );
        assert_eq!(
            ledger.unresolved_uploads(None, 10).await.unwrap(),
            vec![failed.media]
        );
        assert_eq!(
            ledger
                .inspect_unresolved(&storage, failed.media)
                .await
                .unwrap(),
            UploadObservation::Missing
        );
        assert!(ledger
            .recover_settled_upload(&storage, failed.media)
            .await
            .is_err());
        assert_eq!(
            ledger.upload_state(failed.media).await.unwrap(),
            Some(UploadState::Uploading)
        );
        ledger.abandon_upload(failed.media).await.unwrap();
        assert_eq!(
            ledger.upload_state(failed.media).await.unwrap(),
            Some(UploadState::Abandoned)
        );
        assert_eq!(
            ledger.orphan_candidates(None, 10).await.unwrap(),
            vec![failed.media]
        );
        assert_eq!(
            ledger
                .cleanup_abandoned(&storage, failed.media)
                .await
                .unwrap(),
            AbandonedCleanup::Missing
        );
        let file = ledger
            .stage_reserved(&storage, owner, "note.txt", b"hello", i64::MAX)
            .await
            .unwrap();
        let media = MediaRef { owner, id: file.id };
        assert!(storage
            .save_reserved(media, "note.txt", b"overwrite", i64::MAX)
            .await
            .is_err());
        assert_eq!(storage.read(owner, file.id).await.unwrap().1, b"hello");
        let tx = db.begin().await.unwrap();
        assert!(ReferenceLedger::attach(&tx, media).await.is_err());
        tx.rollback().await.unwrap();
        assert_eq!(
            ledger.upload_state(media).await.unwrap(),
            Some(UploadState::Staged)
        );
        assert_eq!(
            ledger.cleanup_abandoned(&storage, media).await.unwrap(),
            AbandonedCleanup::Unknown
        );
        let tx = db.begin().await.unwrap();
        ReferenceLedger::claim_reserved(&tx, media).await.unwrap();
        tx.commit().await.unwrap();
        assert_eq!(
            ledger.upload_state(media).await.unwrap(),
            Some(UploadState::Committed)
        );
        assert!(ledger.abandon_upload(media).await.is_err());
        assert_eq!(
            ledger.cleanup_abandoned(&storage, media).await.unwrap(),
            AbandonedCleanup::Referenced
        );
        let interrupted = MediaRef {
            owner,
            id: Uuid::new_v4(),
        };
        let intent = UploadIntent::new("recover.txt", b"recover", i64::MAX).unwrap();
        ledger.begin_upload(interrupted, &intent).await.unwrap();
        assert_eq!(
            ledger.upload_intent(interrupted).await.unwrap(),
            Some(intent)
        );
        let written = storage
            .save_reserved(interrupted, "recover.txt", b"recover", i64::MAX)
            .await
            .unwrap();
        assert_eq!(
            ledger.unresolved_uploads(None, 10).await.unwrap(),
            vec![interrupted]
        );
        assert!(ledger.unresolved_uploads(None, 0).await.is_err());
        assert!(ledger.unresolved_uploads(None, 1001).await.is_err());
        assert!(ledger
            .unresolved_uploads(Some(interrupted), 1)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            ledger
                .inspect_unresolved(&storage, interrupted)
                .await
                .unwrap(),
            UploadObservation::Present(written.clone())
        );
        assert_eq!(
            ledger
                .cleanup_abandoned(&storage, interrupted)
                .await
                .unwrap(),
            AbandonedCleanup::Unknown
        );
        assert_eq!(
            ledger
                .recover_settled_upload(&storage, interrupted)
                .await
                .unwrap(),
            written
        );
        assert_eq!(
            ledger.upload_state(interrupted).await.unwrap(),
            Some(UploadState::Staged)
        );
        assert_eq!(
            ledger
                .inspect_unresolved(&storage, interrupted)
                .await
                .unwrap(),
            UploadObservation::NotUploading
        );
        assert!(ledger
            .unresolved_uploads(None, 10)
            .await
            .unwrap()
            .is_empty());
        ledger.abandon_upload(interrupted).await.unwrap();
        assert_eq!(
            ledger
                .cleanup_abandoned(&storage, interrupted)
                .await
                .unwrap(),
            AbandonedCleanup::Removed
        );
        // A self-consistent but different payload must not be recovered.
        let mismatch = MediaRef {
            owner,
            id: Uuid::new_v4(),
        };
        let intent = UploadIntent::new("recover.txt", b"correct", i64::MAX).unwrap();
        ledger.begin_upload(mismatch, &intent).await.unwrap();
        storage
            .save_reserved(mismatch, "recover.txt", b"another", i64::MAX)
            .await
            .unwrap();
        assert!(matches!(
            ledger.inspect_unresolved(&storage, mismatch).await.unwrap(),
            UploadObservation::Present(_)
        ));
        assert!(ledger
            .recover_settled_upload(&storage, mismatch)
            .await
            .is_err());
        assert_eq!(
            ledger.upload_state(mismatch).await.unwrap(),
            Some(UploadState::Uploading)
        );
        assert_eq!(
            ledger.cleanup_abandoned(&storage, mismatch).await.unwrap(),
            AbandonedCleanup::Unknown
        );
        // Simulate an older tracked upload with no original intent record.
        let tx = db.begin().await.unwrap();
        let query = statement(
            &tx,
            "DELETE FROM _fexu_media_upload_intents WHERE owner = ? AND file_id = ?",
            "DELETE FROM _fexu_media_upload_intents WHERE owner = $1 AND file_id = $2",
            mismatch,
        )
        .unwrap();
        change_one(&tx, query).await.unwrap();
        tx.commit().await.unwrap();
        assert_eq!(ledger.upload_intent(mismatch).await.unwrap(), None);
        assert!(ledger
            .recover_settled_upload(&storage, mismatch)
            .await
            .is_err());
        // Once discard is requested, even a delayed successful writer or
        // operator recovery cannot move the upload to Staged.
        let pending = MediaRef {
            owner,
            id: Uuid::new_v4(),
        };
        let intent = UploadIntent::new("pending.txt", b"pending", i64::MAX).unwrap();
        ledger.begin_upload(pending, &intent).await.unwrap();
        ledger.request_upload_fence(pending).await.unwrap();
        ledger.request_upload_fence(pending).await.unwrap();
        assert!(ledger.upload_fence_requested(pending).await.unwrap());
        assert!(ledger.finish_upload(pending).await.is_err());
        assert!(ledger.abandon_upload(pending).await.is_err());
        storage
            .save_reserved(pending, "pending.txt", b"pending", i64::MAX)
            .await
            .unwrap();
        assert!(ledger
            .recover_settled_upload(&storage, pending)
            .await
            .is_err());
        assert_eq!(
            ledger.upload_state(pending).await.unwrap(),
            Some(UploadState::Uploading)
        );
        assert_eq!(
            ledger.cleanup_abandoned(&storage, pending).await.unwrap(),
            AbandonedCleanup::Unknown
        );
        assert!(ledger.request_upload_fence(media).await.is_err());
        let local = MediaRef {
            owner,
            id: Uuid::new_v4(),
        };
        let intent = UploadIntent::new("note.txt", b"late", i64::MAX).unwrap();
        ledger.begin_upload(local, &intent).await.unwrap();
        let other = LocalStorage::open(&root, 64, &["text/plain"]).unwrap();
        let slot = root
            .join(owner.to_string())
            .join(format!("{}.write-lock", local.id));
        std::fs::write(&slot, b"").unwrap();
        assert!(ledger
            .recover_settled_local_discard(&storage, local)
            .await
            .is_err());
        assert!(ledger
            .abandon_unresolved_local(&storage, local)
            .await
            .is_err());
        assert!(ledger.local_fence_requested(local).await.unwrap());
        assert!(ledger.finish_upload(local).await.is_err());
        assert!(ledger.abandon_upload(local).await.is_err());
        assert!(ledger.request_upload_fence(local).await.is_err());
        assert!(other
            .save_reserved(local, "note.txt", b"late", i64::MAX)
            .await
            .is_err());
        // Offline maintenance: simulate a crashed writer whose slot remains.
        std::fs::write(&slot, b"FX").unwrap();
        assert!(ledger
            .recover_settled_local_discard(&storage, local)
            .await
            .is_err());
        assert_eq!(std::fs::read(&slot).unwrap(), b"FX");
        assert_eq!(
            ledger.upload_state(local).await.unwrap(),
            Some(UploadState::Uploading)
        );
        std::fs::write(&slot, b"").unwrap();
        ledger
            .recover_settled_local_discard(&storage, local)
            .await
            .unwrap();
        ledger
            .recover_settled_local_discard(&other, local)
            .await
            .unwrap();
        ledger
            .abandon_unresolved_local(&other, local)
            .await
            .unwrap();
        assert_eq!(
            ledger.upload_state(local).await.unwrap(),
            Some(UploadState::Abandoned)
        );
        assert!(other
            .save_reserved(local, "note.txt", b"late", i64::MAX)
            .await
            .is_err());
        assert_eq!(
            ledger.cleanup_local_residue(&storage, local).await.unwrap(),
            AbandonedCleanup::Missing
        );
        assert_eq!(std::fs::read(&slot).unwrap(), b"FXL1");
        let limited = Uuid::new_v4();
        ledger.configure_owner_quota(limited, 2, 8).await.unwrap();
        let other = ReferenceLedger::new(db.clone());
        let (first, second) = tokio::join!(
            ledger.stage_reserved(&storage, limited, "a.txt", b"12345", i64::MAX),
            other.stage_reserved(&storage, limited, "b.txt", b"12345", i64::MAX)
        );
        let (winner, denied) = match (first, second) {
            (Ok(file), Err(error)) | (Err(error), Ok(file)) => (file, error),
            _ => panic!("only one upload may fit the budget"),
        };
        assert_eq!(denied.error, StorageError::QuotaExceeded);
        assert_eq!(ledger.upload_state(denied.media).await.unwrap(), None);
        assert_eq!(
            other.owner_quota(limited).await.unwrap(),
            Some(OwnerQuota {
                max_files: 2,
                max_bytes: 8,
                reserved_files: 1,
                reserved_bytes: 5,
            })
        );
        assert!(ledger.configure_owner_quota(limited, 3, 99).await.is_err());
        let failed_media = MediaRef {
            owner: limited,
            id: Uuid::new_v4(),
        };
        let intent = UploadIntent::new("pending.txt", b"123", i64::MAX).unwrap();
        ledger.begin_upload(failed_media, &intent).await.unwrap();
        let denied = other
            .stage_reserved(&storage, limited, "c.txt", b"x", i64::MAX)
            .await
            .unwrap_err();
        assert_eq!(denied.error, StorageError::QuotaExceeded);
        assert_eq!(other.upload_state(denied.media).await.unwrap(), None);
        assert!(!ledger.release_settled_quota(failed_media).await.unwrap());
        ledger.abandon_upload(failed_media).await.unwrap();
        assert!(!ledger.release_settled_quota(failed_media).await.unwrap());
        assert_eq!(
            ledger
                .cleanup_abandoned_page(&storage, None, 100)
                .await
                .unwrap()
                .iter()
                .find(|item| item.media == failed_media)
                .unwrap()
                .outcome,
            Ok(AbandonedCleanup::Missing)
        );
        let quota = other.owner_quota(limited).await.unwrap().unwrap();
        assert_eq!(quota.reserved_files, 2);
        assert_eq!(quota.reserved_bytes, 8);
        assert!(storage.exists(limited, winner.id).await.unwrap());
        let (left, right) = tokio::join!(
            ledger.release_settled_quota(failed_media),
            other.release_settled_quota(failed_media)
        );
        assert_ne!(left.unwrap(), right.unwrap());
        assert!(!ledger.release_settled_quota(failed_media).await.unwrap());
        let quota = other.owner_quota(limited).await.unwrap().unwrap();
        assert_eq!(quota.reserved_files, 1);
        assert_eq!(quota.reserved_bytes, 5);
        let winner_media = MediaRef {
            owner: limited,
            id: winner.id,
        };
        assert!(!ledger.release_settled_quota(winner_media).await.unwrap());
        let next = ledger
            .stage_reserved(&storage, limited, "next.txt", b"123", i64::MAX)
            .await
            .unwrap();
        let next_media = MediaRef {
            owner: limited,
            id: next.id,
        };
        // Simulate an older reservation with no per-ID charge journal.
        let query = statement(
            db,
            "DELETE FROM _fexu_media_quota_charges WHERE owner = ? AND file_id = ?",
            "DELETE FROM _fexu_media_quota_charges WHERE owner = $1 AND file_id = $2",
            next_media,
        )
        .unwrap();
        db.execute(query).await.unwrap();
        ledger.abandon_upload(next_media).await.unwrap();
        ledger
            .cleanup_abandoned_page(&storage, None, 100)
            .await
            .unwrap();
        assert!(ledger.cleanup_receipt(next_media).await.unwrap().is_some());
        assert!(!ledger.release_settled_quota(next_media).await.unwrap());
        let quota = other.owner_quota(limited).await.unwrap().unwrap();
        assert_eq!(quota.reserved_files, 2);
        assert_eq!(quota.reserved_bytes, 8);
        // Local release must recheck residue even when a receipt exists.
        let local_owner = Uuid::new_v4();
        ledger
            .configure_owner_quota(local_owner, 1, 3)
            .await
            .unwrap();
        let local_file = ledger
            .stage_reserved(&storage, local_owner, "local.txt", b"123", i64::MAX)
            .await
            .unwrap();
        let local_media = MediaRef {
            owner: local_owner,
            id: local_file.id,
        };
        assert!(!ledger
            .cleanup_and_release_local_quota(&storage, local_media)
            .await
            .unwrap());
        ledger.abandon_upload(local_media).await.unwrap();
        ledger
            .cleanup_abandoned_page(&storage, None, 100)
            .await
            .unwrap();
        assert!(ledger.cleanup_receipt(local_media).await.unwrap().is_some());
        let directory = root.join(local_owner.to_string());
        let slot = directory.join(format!("{}.write-lock", local_media.id));
        let residue = directory.join(format!("{}.partial", local_media.id));
        std::fs::write(&slot, b"").unwrap();
        std::fs::write(&residue, b"123").unwrap();
        assert!(ledger
            .cleanup_and_release_local_quota(&storage, local_media)
            .await
            .is_err());
        assert!(residue.exists());
        let quota = ledger.owner_quota(local_owner).await.unwrap().unwrap();
        assert_eq!(quota.reserved_bytes, 3);
        // Simulate the writer settling and releasing its own slot.
        std::fs::remove_file(&slot).unwrap();
        let (left, right) = tokio::join!(
            ledger.cleanup_and_release_local_quota(&storage, local_media),
            other.cleanup_and_release_local_quota(&storage, local_media)
        );
        assert_ne!(left.unwrap(), right.unwrap());
        assert!(!residue.exists());
        assert_eq!(std::fs::read(&slot).unwrap(), b"FXL1");
        assert!(!ledger
            .cleanup_and_release_local_quota(&storage, local_media)
            .await
            .unwrap());
        let quota = ledger.owner_quota(local_owner).await.unwrap().unwrap();
        assert_eq!(quota.reserved_files, 0);
        assert_eq!(quota.reserved_bytes, 0);
        assert!(storage
            .save_reserved(local_media, "late.txt", b"123", i64::MAX)
            .await
            .is_err());
        assert!(!ledger
            .cleanup_and_release_local_quota(&storage, next_media)
            .await
            .unwrap());
        // A missing journal must remain visible without changing the budget.
        let audit = ledger.audit_owner_quota(limited).await.unwrap().unwrap();
        assert_eq!(audit.quota.reserved_files, 2);
        assert_eq!(audit.quota.reserved_bytes, 8);
        assert_eq!(audit.journaled_files, 1);
        assert_eq!(audit.journaled_bytes, 5);
        assert_eq!(audit.released_files, 1);
        assert_eq!(audit.unjournaled_uploads, 1);
        assert_eq!(audit.legacy_references, 0);
        assert!(!audit.counters_match_journal());
        assert_eq!(
            ledger.owner_quota(limited).await.unwrap(),
            Some(audit.quota)
        );

        let audit = ledger
            .audit_owner_quota(local_owner)
            .await
            .unwrap()
            .unwrap();
        assert!(audit.counters_match_journal());
        assert_eq!(audit.journaled_files, 0);
        assert_eq!(audit.journaled_bytes, 0);
        assert_eq!(audit.released_files, 1);
        let legacy = MediaRef {
            owner: local_owner,
            id: Uuid::new_v4(),
        };
        ledger.register(legacy).await.unwrap();
        let audit = ledger
            .audit_owner_quota(local_owner)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(audit.legacy_references, 1);
        assert!(audit.counters_match_journal());
        let absent = Uuid::new_v4();
        assert_eq!(ledger.audit_owner_quota(absent).await.unwrap(), None);
        assert_eq!(ledger.owner_quota(absent).await.unwrap(), None);

        // Resize preserves even the historical unjournaled charge.
        assert!(!ledger.resize_owner_quota(limited, 1, 8).await.unwrap());
        assert!(!ledger.resize_owner_quota(limited, 2, 7).await.unwrap());
        assert!(ledger.resize_owner_quota(limited, -1, 8).await.is_err());
        assert_eq!(
            other.owner_quota(limited).await.unwrap(),
            Some(OwnerQuota {
                max_files: 2,
                max_bytes: 8,
                reserved_files: 2,
                reserved_bytes: 8,
            })
        );
        assert!(ledger.resize_owner_quota(limited, 3, 10).await.unwrap());
        let quota = other.owner_quota(limited).await.unwrap().unwrap();
        assert_eq!(quota.max_files, 3);
        assert_eq!(quota.max_bytes, 10);
        assert_eq!(quota.reserved_files, 2);
        assert_eq!(quota.reserved_bytes, 8);
        assert!(ledger.resize_owner_quota(limited, 2, 8).await.unwrap());
        let absent = Uuid::new_v4();
        assert!(!ledger.resize_owner_quota(absent, 1, 1).await.unwrap());
        assert_eq!(ledger.owner_quota(absent).await.unwrap(), None);

        // A concurrent admission and shrink must never both succeed.
        let racing = Uuid::new_v4();
        ledger.configure_owner_quota(racing, 1, 3).await.unwrap();
        let (resized, admitted) = tokio::join!(
            ledger.resize_owner_quota(racing, 0, 0),
            other.stage_reserved(&storage, racing, "race.txt", b"123", i64::MAX)
        );
        let resized = resized.unwrap();
        assert_ne!(resized, admitted.is_ok());
        let quota = ledger.owner_quota(racing).await.unwrap().unwrap();
        if resized {
            assert_eq!(admitted.unwrap_err().error, StorageError::QuotaExceeded);
            assert_eq!(quota.max_files, 0);
            assert_eq!(quota.reserved_files, 0);
        } else {
            assert_eq!(quota.max_files, 1);
            assert_eq!(quota.reserved_files, 1);
            assert_eq!(quota.reserved_bytes, 3);
        }
        // Historical journal repair never modifies existing counters.
        assert!(!ledger
            .journal_historical_quota(next_media, 2)
            .await
            .unwrap());
        assert!(!ledger
            .journal_historical_quota(winner_media, 5)
            .await
            .unwrap());
        assert!(!ledger
            .journal_historical_quota(failed_media, 3)
            .await
            .unwrap());
        let (left, right) = tokio::join!(
            ledger.journal_historical_quota(next_media, 3),
            other.journal_historical_quota(next_media, 3)
        );
        assert_ne!(left.unwrap(), right.unwrap());
        let audit = ledger.audit_owner_quota(limited).await.unwrap().unwrap();
        assert!(audit.counters_match_journal());
        assert_eq!(audit.quota.reserved_files, 2);
        assert_eq!(audit.quota.reserved_bytes, 8);
        assert_eq!(audit.unjournaled_uploads, 0);
        assert!(ledger
            .cleanup_and_release_local_quota(&storage, next_media)
            .await
            .unwrap());
        assert!(!ledger
            .journal_historical_quota(next_media, 3)
            .await
            .unwrap());
        let audit = ledger.audit_owner_quota(limited).await.unwrap().unwrap();
        assert!(audit.counters_match_journal());
        assert_eq!(audit.quota.reserved_files, 1);
        assert_eq!(audit.quota.reserved_bytes, 5);
        // Legacy adoption charges a verified retained file exactly once.
        let legacy_owner = Uuid::new_v4();
        ledger
            .configure_owner_quota(legacy_owner, 1, 4)
            .await
            .unwrap();
        let file = storage
            .save(legacy_owner, "legacy.txt", b"data")
            .await
            .unwrap();
        let media = MediaRef {
            owner: legacy_owner,
            id: file.id,
        };
        ledger.register(media).await.unwrap();
        assert!(!ledger.adopt_legacy_quota(&storage, media).await.unwrap());
        let tx = db.begin().await.unwrap();
        ReferenceLedger::attach(&tx, media).await.unwrap();
        tx.commit().await.unwrap();
        let (left, right) = tokio::join!(
            ledger.adopt_legacy_quota(&storage, media),
            other.adopt_legacy_quota(&storage, media)
        );
        assert_ne!(left.unwrap(), right.unwrap());
        assert_eq!(
            ledger.upload_state(media).await.unwrap(),
            Some(UploadState::Committed)
        );
        assert_eq!(ledger.upload_intent(media).await.unwrap(), None);
        let audit = ledger
            .audit_owner_quota(legacy_owner)
            .await
            .unwrap()
            .unwrap();
        assert!(audit.counters_match_journal());
        assert_eq!(audit.quota.reserved_files, 1);
        assert_eq!(audit.quota.reserved_bytes, 4);
        assert_eq!(audit.legacy_references, 0);
        let extra = storage
            .save(legacy_owner, "extra.txt", b"x")
            .await
            .unwrap();
        let extra_media = MediaRef {
            owner: legacy_owner,
            id: extra.id,
        };
        ledger.register(extra_media).await.unwrap();
        let tx = db.begin().await.unwrap();
        ReferenceLedger::attach(&tx, extra_media).await.unwrap();
        tx.commit().await.unwrap();
        assert_eq!(
            ledger.adopt_legacy_quota(&storage, extra_media).await,
            Err(StorageError::QuotaExceeded)
        );
        assert_eq!(ledger.upload_state(extra_media).await.unwrap(), None);
        let tx = db.begin().await.unwrap();
        ReferenceLedger::detach(&tx, media).await.unwrap();
        assert!(ReferenceLedger::admit_detached_cleanup(&tx, media)
            .await
            .unwrap());
        tx.commit().await.unwrap();
        assert!(ledger
            .cleanup_and_release_local_quota(&storage, media)
            .await
            .unwrap());
        assert!(ledger
            .adopt_legacy_quota(&storage, extra_media)
            .await
            .unwrap());
        let audit = ledger
            .audit_owner_quota(legacy_owner)
            .await
            .unwrap()
            .unwrap();
        assert!(audit.counters_match_journal());
        assert_eq!(audit.quota.reserved_files, 1);
        assert_eq!(audit.quota.reserved_bytes, 1);
        let zero = Uuid::new_v4();
        ledger.configure_owner_quota(zero, 0, 0).await.unwrap();
        assert_eq!(
            ledger
                .stage_reserved(&storage, zero, "no.txt", b"x", i64::MAX)
                .await
                .unwrap_err()
                .error,
            StorageError::QuotaExceeded
        );
        assert!(ledger.configure_owner_quota(owner, 10, 100).await.is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn sqlite_reserved_upload_contract() {
        let db = fexu_db::connect("sqlite::memory:").await.unwrap();
        reserved_contract(&db).await;
        db.close().await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires disposable PostgreSQL"]
    async fn postgres_reserved_upload_contract() {
        let url = std::env::var("DATABASE_URL").unwrap();
        let fixture = fexu_db::DisposablePostgres::create(&url).await.unwrap();
        let db = fexu_db::connect(fixture.url()).await.unwrap();
        reserved_contract(&db).await;
        db.close().await.unwrap();
        fixture.drop_database().await.unwrap();
    }

    async fn abandoned_page_contract(db: &DatabaseConnection) {
        use crate::{LocalStorage, Storage};

        let root = std::env::temp_dir().join(format!("fexu-page-{}", Uuid::new_v4()));
        let storage = LocalStorage::open(&root, 64, &["text/plain"]).unwrap();
        let ledger = ReferenceLedger::new(db.clone());
        ledger.initialize().await.unwrap();
        let owner = Uuid::new_v4();
        let mut abandoned = Vec::new();
        for _ in 0..2 {
            let file = ledger
                .stage_reserved(&storage, owner, "note.txt", b"discard", i64::MAX)
                .await
                .unwrap();
            let media = MediaRef { owner, id: file.id };
            ledger.abandon_upload(media).await.unwrap();
            abandoned.push(media);
        }
        abandoned.sort_by_key(|item| item.id);
        let staged = ledger
            .stage_reserved(&storage, owner, "note.txt", b"keep", i64::MAX)
            .await
            .unwrap();
        let committed = ledger
            .stage_reserved(&storage, owner, "note.txt", b"model", i64::MAX)
            .await
            .unwrap();
        let media = MediaRef {
            owner,
            id: committed.id,
        };
        let tx = db.begin().await.unwrap();
        ReferenceLedger::claim_reserved(&tx, media).await.unwrap();
        ReferenceLedger::detach(&tx, media).await.unwrap();
        tx.commit().await.unwrap();
        let legacy = storage.save(owner, "note.txt", b"legacy").await.unwrap();
        ledger
            .register(MediaRef {
                owner,
                id: legacy.id,
            })
            .await
            .unwrap();
        let uploading = MediaRef {
            owner,
            id: Uuid::new_v4(),
        };
        let intent = UploadIntent::new("note.txt", b"pending", i64::MAX).unwrap();
        ledger.begin_upload(uploading, &intent).await.unwrap();

        assert!(ledger.abandoned_candidates(None, 0).await.is_err());
        assert!(ledger.abandoned_candidates(None, 1001).await.is_err());
        assert_eq!(
            ledger.abandoned_candidates(None, 10).await.unwrap(),
            abandoned
        );
        let page = ledger
            .cleanup_abandoned_page(&storage, None, 1)
            .await
            .unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].media, abandoned[0]);
        assert_eq!(page[0].outcome, Ok(AbandonedCleanup::Removed));
        let next = ledger
            .cleanup_abandoned_page(&storage, Some(page[0].media), 1)
            .await
            .unwrap();
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].media, abandoned[1]);
        assert_eq!(next[0].outcome, Ok(AbandonedCleanup::Removed));
        let retry = ledger
            .cleanup_abandoned_page(&storage, None, 10)
            .await
            .unwrap();
        assert!(retry.is_empty());
        let reopened = ReferenceLedger::new(db.clone());
        reopened.initialize().await.unwrap();
        for media in &abandoned {
            assert_eq!(
                reopened.cleanup_receipt(*media).await.unwrap(),
                Some(CleanupReceipt::Removed)
            );
        }
        // Simulate interruption after retirement and storage deletion but
        // before recording a receipt. The next page records Missing.
        let missing = MediaRef {
            owner,
            id: Uuid::new_v4(),
        };
        let intent = UploadIntent::new("note.txt", b"pending", i64::MAX).unwrap();
        ledger.begin_upload(missing, &intent).await.unwrap();
        ledger.abandon_upload(missing).await.unwrap();
        assert_eq!(
            ledger.check(missing).await.unwrap(),
            ReferenceDecision::Unreferenced(())
        );
        assert_eq!(ledger.cleanup_receipt(missing).await.unwrap(), None);
        let retry = reopened
            .cleanup_abandoned_page(&storage, None, 10)
            .await
            .unwrap();
        assert_eq!(retry.len(), 1);
        assert_eq!(retry[0].media, missing);
        assert_eq!(retry[0].outcome, Ok(AbandonedCleanup::Missing));
        assert_eq!(
            reopened.cleanup_receipt(missing).await.unwrap(),
            Some(CleanupReceipt::Missing)
        );
        assert!(reopened
            .abandoned_candidates(None, 10)
            .await
            .unwrap()
            .is_empty());
        let tx = db.begin().await.unwrap();
        assert!(ReferenceLedger::attach(&tx, missing).await.is_err());
        tx.rollback().await.unwrap();
        assert!(storage.exists(owner, staged.id).await.unwrap());
        assert!(storage.exists(owner, committed.id).await.unwrap());
        assert!(storage.exists(owner, legacy.id).await.unwrap());
        assert_eq!(
            ledger.upload_state(uploading).await.unwrap(),
            Some(UploadState::Uploading)
        );
        let directory = root.join(owner.to_string());
        let residue = abandoned[0];
        let partial = directory.join(format!("{}.partial", residue.id));
        let temporary = directory.join(format!("{}.json.tmp", residue.id));
        let sidecar = directory.join(format!("{}.json", residue.id));
        std::fs::write(&partial, b"partial").unwrap();
        std::fs::write(&temporary, b"broken").unwrap();
        std::fs::write(&sidecar, b"truncated").unwrap();
        // Unexpected directories must block the entire preflight.
        let data = directory.join(residue.id.to_string());
        std::fs::create_dir(&data).unwrap();
        assert!(ledger
            .cleanup_local_residue(&storage, residue)
            .await
            .is_err());
        assert!(partial.exists() && temporary.exists() && sidecar.exists());
        std::fs::remove_dir(&data).unwrap();
        std::fs::write(&data, b"orphan").unwrap();
        assert_eq!(
            ledger
                .cleanup_local_residue(&storage, residue)
                .await
                .unwrap(),
            AbandonedCleanup::Removed
        );
        assert!(!partial.exists() && !temporary.exists() && !sidecar.exists() && !data.exists());
        assert_eq!(
            ledger
                .cleanup_local_residue(&storage, residue)
                .await
                .unwrap(),
            AbandonedCleanup::Missing
        );
        assert_eq!(
            ledger.cleanup_receipt(residue).await.unwrap(),
            Some(CleanupReceipt::Removed)
        );
        let staged_media = MediaRef {
            owner,
            id: staged.id,
        };
        assert_eq!(
            ledger
                .cleanup_local_residue(&storage, staged_media)
                .await
                .unwrap(),
            AbandonedCleanup::Unknown
        );
        assert!(storage.exists(owner, staged.id).await.unwrap());
        assert_eq!(
            ledger
                .cleanup_local_residue(&storage, uploading)
                .await
                .unwrap(),
            AbandonedCleanup::Unknown
        );
        assert!(storage.exists(owner, legacy.id).await.unwrap());
        let mut work = Vec::new();
        for _ in 0..3 {
            let file = ledger
                .stage_reserved(&storage, owner, "note.txt", b"worker", i64::MAX)
                .await
                .unwrap();
            let media = MediaRef { owner, id: file.id };
            ledger.abandon_upload(media).await.unwrap();
            work.push(media);
        }
        work.sort_by_key(|item| item.id);
        assert!(ledger
            .cleanup_abandoned_batch(&storage, None, 1000, 2)
            .await
            .is_err());
        assert!(ledger
            .cleanup_abandoned_batch(&storage, None, 1, 0)
            .await
            .is_err());
        assert!(ledger
            .cleanup_abandoned_batch(&storage, None, 0, 1)
            .await
            .is_err());
        assert!(ledger
            .cleanup_abandoned_batch(&storage, None, 1, 17)
            .await
            .is_err());
        let batch = ledger
            .cleanup_abandoned_batch(&storage, None, 1, 2)
            .await
            .unwrap();
        assert_eq!(batch.items.len(), 2);
        assert_eq!(batch.items[0].media, work[0]);
        assert_eq!(batch.items[1].media, work[1]);
        assert_eq!(batch.next_after, Some(work[1]));
        assert!(!batch.reached_end);
        assert!(batch.scan_error.is_none());
        assert!(batch
            .items
            .iter()
            .all(|item| item.outcome == Ok(AbandonedCleanup::Removed)));
        let batch = reopened
            .cleanup_abandoned_batch(&storage, batch.next_after, 2, 2)
            .await
            .unwrap();
        assert_eq!(batch.items.len(), 1);
        assert_eq!(batch.items[0].media, work[2]);
        assert_eq!(batch.items[0].outcome, Ok(AbandonedCleanup::Removed));
        assert!(batch.reached_end);
        assert_eq!(batch.next_after, None);
        assert!(batch.scan_error.is_none());
        let batch = ledger
            .cleanup_abandoned_batch(&storage, None, 2, 2)
            .await
            .unwrap();
        assert!(batch.items.is_empty() && batch.reached_end);
        assert!(storage.exists(owner, staged.id).await.unwrap());
        assert!(storage.exists(owner, committed.id).await.unwrap());
        assert!(storage.exists(owner, legacy.id).await.unwrap());
        // Explicit opt-in only: existing committed zero-reference files
        // are not automatically selected by the worker.
        let detached = MediaRef {
            owner,
            id: committed.id,
        };
        let tx = db.begin().await.unwrap();
        ReferenceLedger::attach(&tx, detached).await.unwrap();
        ReferenceLedger::attach(&tx, detached).await.unwrap();
        ReferenceLedger::detach(&tx, detached).await.unwrap();
        assert!(!ReferenceLedger::admit_detached_cleanup(&tx, detached)
            .await
            .unwrap());
        ReferenceLedger::detach(&tx, detached).await.unwrap();
        assert!(ReferenceLedger::admit_detached_cleanup(&tx, detached)
            .await
            .unwrap());
        tx.rollback().await.unwrap();
        assert_eq!(
            ledger.upload_state(detached).await.unwrap(),
            Some(UploadState::Committed)
        );
        let tx = db.begin().await.unwrap();
        ReferenceLedger::attach(&tx, detached).await.unwrap();
        ReferenceLedger::detach(&tx, detached).await.unwrap();
        assert!(!ReferenceLedger::admit_detached_cleanup(&tx, staged_media)
            .await
            .unwrap());
        assert!(!ReferenceLedger::admit_detached_cleanup(&tx, uploading)
            .await
            .unwrap());
        assert!(ReferenceLedger::admit_detached_cleanup(&tx, detached)
            .await
            .unwrap());
        tx.commit().await.unwrap();
        assert_eq!(
            ledger.upload_state(detached).await.unwrap(),
            Some(UploadState::Abandoned)
        );
        let tx = db.begin().await.unwrap();
        assert!(ReferenceLedger::attach(&tx, detached).await.is_err());
        tx.rollback().await.unwrap();
        let batch = ledger
            .cleanup_abandoned_batch(&storage, None, 10, 1)
            .await
            .unwrap();
        assert_eq!(batch.items.len(), 1);
        assert_eq!(batch.items[0].media, detached);
        assert_eq!(batch.items[0].outcome, Ok(AbandonedCleanup::Removed));
        assert!(!storage.exists(owner, committed.id).await.unwrap());
        assert!(storage.exists(owner, staged.id).await.unwrap());
        assert!(storage.exists(owner, legacy.id).await.unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn sqlite_abandoned_page_contract() {
        let db = fexu_db::connect("sqlite::memory:").await.unwrap();
        abandoned_page_contract(&db).await;
        db.close().await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires disposable PostgreSQL"]
    async fn postgres_abandoned_page_contract() {
        let url = std::env::var("DATABASE_URL").unwrap();
        let fixture = fexu_db::DisposablePostgres::create(&url).await.unwrap();
        let db = fexu_db::connect(fixture.url()).await.unwrap();
        abandoned_page_contract(&db).await;
        db.close().await.unwrap();
        fixture.drop_database().await.unwrap();
    }

    async fn orphan_contract(db: &DatabaseConnection) {
        use crate::{LocalStorage, Storage};

        let root = std::env::temp_dir().join(format!("fexu-orphans-{}", Uuid::new_v4()));
        let storage = LocalStorage::open(&root, 1024, &["text/plain"]).unwrap();
        let ledger = ReferenceLedger::new(db.clone());
        ledger.initialize().await.unwrap();
        let owner = Uuid::new_v4();
        let mut media = Vec::new();
        for _ in 0..4 {
            let file = storage.save(owner, "note.txt", b"orphan").await.unwrap();
            media.push(MediaRef { owner, id: file.id });
        }
        media.sort_by_key(|item| item.id);
        for item in &media[..3] {
            ledger.register(*item).await.unwrap();
        }
        let tx = db.begin().await.unwrap();
        ReferenceLedger::attach(&tx, media[1]).await.unwrap();
        tx.commit().await.unwrap();

        assert!(ledger.orphan_candidates(None, 0).await.is_err());
        assert!(ledger.orphan_candidates(None, 1001).await.is_err());
        let first = ledger.orphan_candidates(None, 1).await.unwrap();
        assert_eq!(first, vec![media[0]]);
        let second = ledger.orphan_candidates(Some(first[0]), 1).await.unwrap();
        assert_eq!(second, vec![media[2]]);
        assert!(ledger
            .orphan_candidates(Some(second[0]), 1)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            ledger.cleanup_abandoned(&storage, media[1]).await.unwrap(),
            AbandonedCleanup::Referenced
        );
        assert_eq!(
            ledger.cleanup_abandoned(&storage, media[3]).await.unwrap(),
            AbandonedCleanup::Unknown
        );
        assert_eq!(
            ledger.cleanup_abandoned(&storage, media[0]).await.unwrap(),
            AbandonedCleanup::Removed
        );
        assert_eq!(
            ledger.cleanup_abandoned(&storage, media[0]).await.unwrap(),
            AbandonedCleanup::Missing
        );
        let tx = db.begin().await.unwrap();
        assert!(ReferenceLedger::attach(&tx, media[0]).await.is_err());
        tx.rollback().await.unwrap();
        assert!(storage.exists(owner, media[1].id).await.unwrap());
        assert!(storage.exists(owner, media[3].id).await.unwrap());
        assert_eq!(
            ledger.orphan_candidates(None, 10).await.unwrap(),
            vec![media[0], media[2]]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn sqlite_orphan_candidates_contract() {
        let db = fexu_db::connect("sqlite::memory:").await.unwrap();
        orphan_contract(&db).await;
        db.close().await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires disposable PostgreSQL"]
    async fn postgres_orphan_candidates_contract() {
        let url = std::env::var("DATABASE_URL").unwrap();
        let fixture = fexu_db::DisposablePostgres::create(&url).await.unwrap();
        let db = fexu_db::connect(fixture.url()).await.unwrap();
        orphan_contract(&db).await;
        db.close().await.unwrap();
        fixture.drop_database().await.unwrap();
    }

    #[tokio::test]
    async fn sqlite_reference_ledger_contract() {
        let path = std::env::temp_dir().join(format!("fexu-ledger-{}.sqlite3", Uuid::new_v4()));
        let url = format!(
            "sqlite://{}?mode=rwc",
            path.to_string_lossy().replace('\\', "/")
        );
        let first = fexu_db::connect(&url).await.unwrap();
        let second = fexu_db::connect(&url).await.unwrap();
        contract(&first, &second).await;
        first.close().await.unwrap();
        second.close().await.unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    #[ignore = "requires disposable PostgreSQL"]
    async fn postgres_reference_ledger_contract() {
        let url = std::env::var("DATABASE_URL").unwrap();
        let fixture = fexu_db::DisposablePostgres::create(&url).await.unwrap();
        let first = fexu_db::connect(fixture.url()).await.unwrap();
        let second = fexu_db::connect(fixture.url()).await.unwrap();
        contract(&first, &second).await;
        first.close().await.unwrap();
        second.close().await.unwrap();
        fixture.drop_database().await.unwrap();
    }
}
