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
        tx.commit().await.map_err(|_| StorageError::Failed)
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
    async fn lock_upload(
        tx: &DatabaseTransaction,
        media: MediaRef,
    ) -> Result<(), StorageError> {
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

    async fn has_fence(
        db: &impl ConnectionTrait,
        media: MediaRef,
    ) -> Result<bool, StorageError> {
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

    /// True once irreversible S3 discard has been requested, even if the
    /// remote marker has not yet been confirmed.
    pub async fn upload_fence_requested(&self, media: MediaRef) -> Result<bool, StorageError> {
        Self::has_fence(&self.db, media).await
    }

    #[cfg(any(feature = "s3-storage", test))]
    async fn request_upload_fence(&self, media: MediaRef) -> Result<(), StorageError> {
        let tx = self.db.begin().await.map_err(|_| StorageError::Failed)?;
        Self::lock_upload(&tx, media).await?;
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
        if Self::has_fence(&tx, media).await? {
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
        if Self::has_fence(&tx, media).await? {
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
