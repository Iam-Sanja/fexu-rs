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
        Ok(())
    }

    /// Enroll a newly allocated immutable ID before its first reference.
    /// An existing or retired ID must never be registered again.
    pub async fn register(&self, media: MediaRef) -> Result<(), StorageError> {
        let query = statement(
            &self.db,
            "INSERT INTO _fexu_media_references VALUES (?, ?, 0, 0)",
            "INSERT INTO _fexu_media_references VALUES ($1, $2, 0, 0)",
            media,
        )?;
        self.db
            .execute(query)
            .await
            .map_err(|_| StorageError::Failed)?;
        Ok(())
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
