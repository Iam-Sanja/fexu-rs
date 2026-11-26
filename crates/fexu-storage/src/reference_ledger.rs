//! Opt-in durable admission gate for cooperating application writers.

use crate::{MediaRef, ReferenceCheck, ReferenceDecision, StorageError};
use async_trait::async_trait;
use fexu_db::driver::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, DatabaseTransaction, Statement,
    TransactionTrait, Value,
};

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

/// Durable write status for uploads enrolled through `stage_reserved`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadState {
    Uploading,
    Staged,
    Committed,
    Abandoned,
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
        Ok(())
    }

    /// Enroll a newly allocated immutable ID before its first reference.
    /// An existing or retired ID must never be registered again.
    pub async fn register(&self, media: MediaRef) -> Result<(), StorageError> {
        register_on(&self.db, media).await
    }

    /// Allocate and durably enroll an immutable ID before writing bytes.
    /// The caller must authorize the upload and exclude cleanup until the
    /// write has settled. Failed writes leave an enrolled candidate.
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
        self.begin_upload(media)
            .await
            .map_err(|error| ReservedUploadError { media, error })?;
        let file = storage
            .save_reserved(media, original_name, bytes, expires_at)
            .await
            .map_err(|error| ReservedUploadError { media, error })?;
        if file.owner != media.owner || file.id != media.id || file.expires_at != Some(expires_at) {
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
    async fn begin_upload(&self, media: MediaRef) -> Result<(), StorageError> {
        let tx = self.db.begin().await.map_err(|_| StorageError::Failed)?;
        register_on(&tx, media).await?;
        let query = statement(
            &tx,
            "INSERT INTO _fexu_media_uploads VALUES (?, ?, 0)",
            "INSERT INTO _fexu_media_uploads VALUES ($1, $2, 0)",
            media,
        )?;
        change_one(&tx, query).await?;
        tx.commit().await.map_err(|_| StorageError::Failed)
    }

    async fn finish_upload(&self, media: MediaRef) -> Result<(), StorageError> {
        let tx = self.db.begin().await.map_err(|_| StorageError::Failed)?;
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
    pub async fn upload_state(
        &self,
        media: MediaRef,
    ) -> Result<Option<UploadState>, StorageError> {
        let query = statement(
            &self.db,
            "SELECT state FROM _fexu_media_uploads WHERE owner = ? AND file_id = ?",
            "SELECT state FROM _fexu_media_uploads WHERE owner = $1 AND file_id = $2",
            media,
        )?;
        let row = self.db.query_one(query).await.map_err(|_| StorageError::Failed)?;
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

    /// Confirm an upload is permanently abandoned. Only call after proving
    /// that all storage writes have settled, including ambiguous responses.
    /// This does not delete bytes or authorize cleanup of referenced files.
    pub async fn abandon_upload(&self, media: MediaRef) -> Result<(), StorageError> {
        let tx = self.db.begin().await.map_err(|_| StorageError::Failed)?;
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
            "UPDATE _fexu_media_uploads SET state = 2
             WHERE owner = ? AND file_id = ? AND state = 1",
            "UPDATE _fexu_media_uploads SET state = 2
             WHERE owner = $1 AND file_id = $2 AND state = 1",
            media,
        )?;
        change_one(tx, query).await?;
        let query = statement(
            tx,
            "UPDATE _fexu_media_references SET refs = 1
             WHERE owner = ? AND file_id = ? AND retired = 0 AND refs = 0",
            "UPDATE _fexu_media_references SET refs = 1
             WHERE owner = $1 AND file_id = $2 AND retired = 0 AND refs = 0",
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
             WHERE owner = ? AND file_id = ? AND retired = 0 AND refs < 9223372036854775807",
            "UPDATE _fexu_media_references SET refs = refs + 1
             WHERE owner = $1 AND file_id = $2 AND retired = 0 AND refs < 9223372036854775807",
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
             WHERE owner = ? AND file_id = ? AND refs = 0 AND retired = 0",
            "UPDATE _fexu_media_references SET retired = 1
             WHERE owner = $1 AND file_id = $2 AND refs = 0 AND retired = 0",
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
        ReferenceLedger::attach(&tx, media).await.unwrap();
        tx.commit().await.unwrap();
        assert_eq!(
            ledger.cleanup_abandoned(&storage, media).await.unwrap(),
            AbandonedCleanup::Referenced
        );
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
