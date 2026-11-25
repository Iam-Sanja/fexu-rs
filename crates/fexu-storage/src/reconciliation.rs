//! Explicit reference-aware reconciliation. No implicit database discovery.

use crate::{MediaRef, StorageError};
use async_trait::async_trait;

/// The application must check every reference to this exact owner/file pair.
#[derive(Debug, PartialEq, Eq)]
pub enum ReferenceDecision<G> {
    Referenced,
    Unknown,
    /// The guard must exclude new references until it is dropped, including
    /// across processes. A snapshot or a process-local mutex is insufficient
    /// when independent database writers exist.
    Unreferenced(G),
}

/// Application-supplied authoritative reference lookup and exclusion mechanism.
///
/// Implementations must fail closed if any model or writer is not covered.
/// A guard must remain valid throughout the storage operation.
#[async_trait]
pub trait ReferenceCheck: Send + Sync {
    type Guard: Send;

    async fn check(
        &self,
        media: MediaRef,
    ) -> Result<ReferenceDecision<Self::Guard>, StorageError>;
}

/// A backend must fence expiry checks against concurrent retention.
/// This is not a general-purpose delete operation.
#[async_trait]
pub trait ExpiryCleanup: Send + Sync {
    async fn cleanup_expired(&self, media: MediaRef) -> Result<bool, StorageError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconciliationOutcome {
    Referenced,
    Unknown,
    Removed,
    NotEligible,
}

/// Reconcile exactly one candidate while holding the application's guard.
///
/// Errors propagate without falling back to unconditional deletion.
/// No scheduled sweep or database transaction is supplied here.
pub async fn reconcile_expired<C, B>(
    references: &C,
    backend: &B,
    media: MediaRef,
) -> Result<ReconciliationOutcome, StorageError>
where
    C: ReferenceCheck + ?Sized,
    B: ExpiryCleanup + ?Sized,
{
    match references.check(media).await? {
        ReferenceDecision::Referenced => Ok(ReconciliationOutcome::Referenced),
        ReferenceDecision::Unknown => Ok(ReconciliationOutcome::Unknown),
        ReferenceDecision::Unreferenced(guard) => {
            let removed = backend.cleanup_expired(media).await;
            drop(guard);
            if removed? {
                Ok(ReconciliationOutcome::Removed)
            } else {
                Ok(ReconciliationOutcome::NotEligible)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use uuid::Uuid;

    struct Guard(Arc<AtomicBool>);

    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.store(false, Ordering::SeqCst);
        }
    }

    struct Check {
        mode: u8,
        held: Arc<AtomicBool>,
        expected: MediaRef,
    }

    #[async_trait]
    impl ReferenceCheck for Check {
        type Guard = Guard;

        async fn check(
            &self,
            media: MediaRef,
        ) -> Result<ReferenceDecision<Self::Guard>, StorageError> {
            assert_eq!(media, self.expected);
            match self.mode {
                0 => Ok(ReferenceDecision::Referenced),
                1 => Ok(ReferenceDecision::Unknown),
                2 => Err(StorageError::Failed),
                _ => {
                    assert!(!self.held.swap(true, Ordering::SeqCst));
                    Ok(ReferenceDecision::Unreferenced(Guard(self.held.clone())))
                }
            }
        }
    }

    struct Backend {
        held: Arc<AtomicBool>,
        calls: AtomicUsize,
        result: Result<bool, ()>,
        expected: MediaRef,
    }

    #[async_trait]
    impl ExpiryCleanup for Backend {
        async fn cleanup_expired(&self, media: MediaRef) -> Result<bool, StorageError> {
            assert_eq!(media, self.expected);
            assert!(self.held.load(Ordering::SeqCst));
            tokio::task::yield_now().await;
            assert!(self.held.load(Ordering::SeqCst));
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.result.map_err(|_| StorageError::Failed)
        }
    }

    #[tokio::test]
    async fn reference_decisions_fail_closed_and_hold_guard_across_io() {
        let media = MediaRef {
            owner: Uuid::new_v4(),
            id: Uuid::new_v4(),
        };
        let held = Arc::new(AtomicBool::new(false));
        let mut check = Check {
            mode: 0,
            held: held.clone(),
            expected: media,
        };
        let mut backend = Backend {
            held: held.clone(),
            calls: AtomicUsize::new(0),
            result: Ok(true),
            expected: media,
        };
        assert_eq!(
            reconcile_expired(&check, &backend, media).await,
            Ok(ReconciliationOutcome::Referenced)
        );
        check.mode = 1;
        assert_eq!(
            reconcile_expired(&check, &backend, media).await,
            Ok(ReconciliationOutcome::Unknown)
        );
        check.mode = 2;
        assert_eq!(
            reconcile_expired(&check, &backend, media).await,
            Err(StorageError::Failed)
        );
        assert_eq!(backend.calls.load(Ordering::SeqCst), 0);

        check.mode = 3;
        assert_eq!(
            reconcile_expired(&check, &backend, media).await,
            Ok(ReconciliationOutcome::Removed)
        );
        assert!(!held.load(Ordering::SeqCst));
        backend.result = Ok(false);
        assert_eq!(
            reconcile_expired(&check, &backend, media).await,
            Ok(ReconciliationOutcome::NotEligible)
        );
        assert!(!held.load(Ordering::SeqCst));
        backend.result = Err(());
        assert_eq!(
            reconcile_expired(&check, &backend, media).await,
            Err(StorageError::Failed)
        );
        assert!(!held.load(Ordering::SeqCst));
        assert_eq!(backend.calls.load(Ordering::SeqCst), 3);
    }
}
