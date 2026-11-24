//! Worker loop. Does not start a web server or apply migrations.

use crate::error::JobError;
use crate::queue::{Finish, Queue};
use crate::registry::{JobContext, JobRegistry};
use fexu_db::driver::{ConnectionTrait, DatabaseBackend};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

pub struct Worker {
    queue: Queue,
    registry: JobRegistry,
    id: String,
    lease_secs: i64,
    poll: Duration,
    concurrency: usize,
}

impl Worker {
    pub fn new(
        queue: Queue,
        registry: JobRegistry,
        lease_secs: i64,
        poll_ms: u64,
        concurrency: u16,
    ) -> Self {
        let concurrency = if queue.database().get_database_backend() == DatabaseBackend::Sqlite {
            1
        } else {
            usize::from(concurrency.clamp(1, 4))
        };
        Self {
            queue,
            registry,
            id: Uuid::new_v4().to_string(),
            lease_secs: lease_secs.clamp(5, 600),
            poll: Duration::from_millis(poll_ms.clamp(50, 60_000)),
            concurrency,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    /// Polls until `stop` is set. PostgreSQL runs up to four handlers concurrently;
    /// SQLite runs one. In-flight handlers finish or reach their timeouts.
    pub async fn run(&self, stop: &AtomicBool) -> Result<(), JobError> {
        self.queue.ready().await?;
        while !stop.load(Ordering::SeqCst) {
            let mut progressed = false;
            // Join all active slots before returning an error or polling again.
            // Dropping a sibling handler early could abandon an external effect.
            let (first, second, third, fourth) = tokio::join!(
                self.run_slot(stop, 0),
                self.run_slot(stop, 1),
                self.run_slot(stop, 2),
                self.run_slot(stop, 3)
            );
            for result in [first, second, third, fourth] {
                match result {
                    Ok(true) => progressed = true,
                    Ok(false) => {}
                    Err(error) if error.code() == "job.locked" => {
                        tracing::warn!(code = "job.locked", "job claim deferred");
                    }
                    Err(error) => return Err(error),
                }
            }
            if self.queue.retain().await.is_err() {
                tracing::warn!(code = "job.database", "job retention skipped");
            }
            if stop.load(Ordering::SeqCst) || progressed {
                continue;
            }
            wait_for_poll(stop, self.poll).await;
        }
        Ok(())
    }

    async fn run_slot(&self, stop: &AtomicBool, slot: usize) -> Result<bool, JobError> {
        if slot >= self.concurrency || stop.load(Ordering::SeqCst) {
            return Ok(false);
        }
        self.run_once().await
    }

    pub async fn run_once(&self) -> Result<bool, JobError> {
        let names = self.registry.names();
        let claimed = self
            .queue
            .claim(&self.id, &names, self.lease_for(&names))
            .await?;
        let Some((record, token)) = claimed else {
            return Ok(false);
        };
        let now = self.queue.clock().unix()?;
        let ctx = JobContext {
            id: record.id,
            attempt: record.attempts,
            now,
            idempotency_key: record.idempotency_key.clone(),
        };
        let timeout = self.registry.timeout_of(&record.handler);
        let outcome = tokio::time::timeout(timeout, self.registry.execute(&record, &ctx)).await;
        let finish = match outcome {
            Ok(Ok(())) => {
                self.queue
                    .finish_success(record.id, &token, &record)
                    .await?
            }
            Ok(Err(error)) if error.is_retry() => {
                self.queue
                    .finish_retry(
                        record.id,
                        &token,
                        record.attempts,
                        record.max_attempts,
                        error.code(),
                    )
                    .await?
            }
            Ok(Err(error)) => {
                self.queue
                    .finish_failed(record.id, &token, error.code())
                    .await?
            }
            Err(_) => {
                self.queue
                    .finish_retry(
                        record.id,
                        &token,
                        record.attempts,
                        record.max_attempts,
                        "job.timeout",
                    )
                    .await?
            }
        };
        if finish == Finish::LostLease {
            tracing::warn!(
                code = "job.lease.lost",
                "stale worker did not finish the job"
            );
        }
        Ok(true)
    }

    fn lease_for(&self, names: &[&str]) -> i64 {
        let mut lease = self.lease_secs;
        for name in names {
            let timeout = i64::try_from(self.registry.timeout_of(name).as_secs()).unwrap_or(30);
            lease = lease.max(timeout.saturating_add(5));
        }
        lease.min(600)
    }
}

pub fn stop_flag() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

/// Wakes within 50ms of `stop` instead of waiting out a long poll interval.
async fn wait_for_poll(stop: &AtomicBool, poll: Duration) {
    let slice = Duration::from_millis(50).min(poll);
    if slice.is_zero() {
        return;
    }
    let mut waited = Duration::ZERO;
    while waited < poll {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let step = slice.min(poll.saturating_sub(waited));
        tokio::time::sleep(step).await;
        waited = waited.saturating_add(step);
    }
}
