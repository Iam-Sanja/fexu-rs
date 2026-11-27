//! Persistent background jobs. The web server does not start the worker.
//!
//! Delivery is **at least once**. Handlers must tolerate repeated execution.
//! [`DELIVERY`] is the only guarantee. An expired lease may cause the same effect
//! to run again. Database transactions cannot guarantee exactly-once external effects
//! such as SMTP. An accepted SMTP response does not prove final delivery.

mod clock;
mod error;
mod host;
mod inspect;
mod mail;
mod queue;
mod registry;
mod schedule;
mod schema;
mod storage;
mod worker;

pub use clock::{Clock, ManualClock, SystemClock};
pub use error::{payload_line, FailureKind, JobError};
pub use host::{WorkerConfig, WorkerHost};
pub use inspect::describe_job;
pub use mail::{enqueue_notice, MailNotice, MailNoticeHandler, MAIL_HANDLER};
pub use queue::{
    delay_secs, EnqueueOutcome, Finish, JobDiagnosis, JobPolicy, JobRecord, JobStatus, JobSummary,
    Queue, When, MAX_ATTEMPTS, MAX_PAYLOAD_BYTES,
};
pub use registry::{JobContext, JobHandler, JobRegistry};
pub use schema::{
    apply_migrations, embedded_migrations, migration_status, migrations_dir,
    render_migration_status, table_ready, JOBS, JOB_HISTORY,
};
pub use storage::{
    enqueue_local_quota_cleanup, enqueue_storage_cleanup, LocalQuotaCleanupHandler, StorageCleanup,
    StorageCleanupHandler, LOCAL_QUOTA_CLEANUP_HANDLER, STORAGE_CLEANUP_HANDLER,
};
pub use worker::{stop_flag, Worker};

/// The queue guarantees at-least-once delivery only, not exactly once.
pub const DELIVERY: &str = "at-least-once";

#[cfg(test)]
mod tests;
