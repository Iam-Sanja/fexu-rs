//! Explicitly registered abandoned-upload cleanup for one storage namespace.

use crate::{EnqueueOutcome, JobContext, JobError, JobHandler, Queue, When};
use async_trait::async_trait;
use fexu_storage::{AbandonedCleanup, LocalStorage, MediaRef, ReferenceLedger, Storage};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const STORAGE_CLEANUP_HANDLER: &str = "fexu_storage_cleanup";

/// No paths, credentials, media IDs or destructive policy in job payloads.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageCleanup {}

/// Bind the handler to the original trusted namespace at worker startup.
/// A single registered handler and worker per namespace is recommended.
pub struct StorageCleanupHandler {
    ledger: ReferenceLedger,
    storage: Arc<dyn Storage>,
    cursor: tokio::sync::Mutex<Option<MediaRef>>,
    page_size: u32,
    max_pages: u32,
}

impl StorageCleanupHandler {
    pub fn new(
        ledger: ReferenceLedger,
        storage: Arc<dyn Storage>,
        page_size: u32,
        max_pages: u32,
    ) -> Result<Self, JobError> {
        if !(1..=16).contains(&max_pages)
            || !(1..=1000).contains(&page_size)
            || page_size * max_pages > 1000
        {
            return Err(JobError::permanent("storage.cleanup.bounds"));
        }
        Ok(Self {
            ledger,
            storage,
            cursor: tokio::sync::Mutex::new(None),
            page_size,
            max_pages,
        })
    }
}

#[async_trait]
impl JobHandler for StorageCleanupHandler {
    const NAME: &'static str = STORAGE_CLEANUP_HANDLER;
    const VERSION: u32 = 1;
    type Payload = StorageCleanup;

    fn timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(120)
    }

    async fn run(&self, _payload: Self::Payload, _ctx: &JobContext) -> Result<(), JobError> {
        let mut cursor = self.cursor.lock().await;
        let batch = self
            .ledger
            .cleanup_abandoned_batch(
                self.storage.as_ref(),
                *cursor,
                self.page_size,
                self.max_pages,
            )
            .await
            .map_err(|_| JobError::retry("storage.cleanup.scan"))?;
        *cursor = batch.next_after;
        let failed = batch
            .items
            .iter()
            .filter(|item| {
                !matches!(
                    &item.outcome,
                    Ok(AbandonedCleanup::Removed | AbandonedCleanup::Missing)
                )
            })
            .count();
        tracing::info!(
            code = "storage.cleanup.batch",
            attempted = batch.items.len(),
            failed,
            reached_end = batch.reached_end,
            scan_failed = batch.scan_error.is_some(),
            "bounded storage cleanup finished"
        );
        if batch.scan_error.is_some() {
            return Err(JobError::retry("storage.cleanup.scan"));
        }
        if failed != 0 {
            return Err(JobError::retry("storage.cleanup.item"));
        }
        Ok(())
    }
}

/// Explicitly enqueue one cleanup job or a recurring schedule.
/// The worker must register a handler bound to the correct namespace.
pub async fn enqueue_storage_cleanup(
    queue: &Queue,
    when: When,
) -> Result<EnqueueOutcome, JobError> {
    queue
        .enqueue::<StorageCleanupHandler>(&StorageCleanup {}, when)
        .await
}


pub const LOCAL_QUOTA_CLEANUP_HANDLER: &str = "fexu_local_quota_cleanup";

/// Bind only to a ledger dedicated to the original local storage namespace.
/// Never register against a mixed-backend or mixed-root ledger.
pub struct LocalQuotaCleanupHandler {
    ledger: ReferenceLedger,
    storage: Arc<LocalStorage>,
    cursor: tokio::sync::Mutex<Option<MediaRef>>,
    page_size: u32,
    max_pages: u32,
}

impl LocalQuotaCleanupHandler {
    pub fn new(
        ledger: ReferenceLedger,
        storage: Arc<LocalStorage>,
        page_size: u32,
        max_pages: u32,
    ) -> Result<Self, JobError> {
        if !(1..=16).contains(&max_pages)
            || !(1..=1000).contains(&page_size)
            || page_size * max_pages > 1000
        {
            return Err(JobError::permanent("storage.quota.bounds"));
        }
        Ok(Self {
            ledger,
            storage,
            cursor: tokio::sync::Mutex::new(None),
            page_size,
            max_pages,
        })
    }
}

#[async_trait]
impl JobHandler for LocalQuotaCleanupHandler {
    const NAME: &'static str = LOCAL_QUOTA_CLEANUP_HANDLER;
    const VERSION: u32 = 1;
    type Payload = StorageCleanup;

    fn timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(120)
    }

    async fn run(&self, _payload: Self::Payload, _ctx: &JobContext) -> Result<(), JobError> {
        let mut cursor = self.cursor.lock().await;
        let batch = self
            .ledger
            .cleanup_local_quota_batch(
                self.storage.as_ref(),
                *cursor,
                self.page_size,
                self.max_pages,
            )
            .await
            .map_err(|_| JobError::retry("storage.quota.scan"))?;
        *cursor = batch.next_after;
        let failed = batch.items.iter().filter(|item| item.outcome.is_err()).count();
        let released = batch
            .items
            .iter()
            .filter(|item| matches!(&item.outcome, Ok(true)))
            .count();
        tracing::info!(
            code = "storage.quota.batch",
            attempted = batch.items.len(),
            released,
            failed,
            reached_end = batch.reached_end,
            scan_failed = batch.scan_error.is_some(),
            "bounded local quota cleanup finished"
        );
        if batch.scan_error.is_some() {
            return Err(JobError::retry("storage.quota.scan"));
        }
        if failed != 0 {
            return Err(JobError::retry("storage.quota.item"));
        }
        Ok(())
    }
}

/// Explicitly enqueue a local quota cleanup job or recurring schedule.
pub async fn enqueue_local_quota_cleanup(
    queue: &Queue,
    when: When,
) -> Result<EnqueueOutcome, JobError> {
    queue
        .enqueue::<LocalQuotaCleanupHandler>(&StorageCleanup {}, when)
        .await
}
