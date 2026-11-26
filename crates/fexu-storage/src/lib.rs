//! Local and optional S3 file storage. Public delivery is disabled by default.
//!
//! [`s3`] validates remote-storage configuration without opening a connection.
//! Malware scanning is not included. Content validation does not replace antivirus scanning
//! and does not decode image pixels.

mod access;
mod fields;
mod http;
mod local;
mod png;
mod reconciliation;
mod reference_ledger;
mod s3;
#[cfg(feature = "s3-storage")]
mod s3_backend;
mod signed;

#[cfg(feature = "s3-storage")]
pub use s3_backend::{S3Limits, S3Storage};

pub use access::{OwnerAccess, StorageAccess, StorageAction};
pub use fields::{
    prepare_media, upload_name_code, AbortReport, BoundMedia, CommitReport, FieldError, FieldSpec,
    IncomingFile, MediaKind, MediaPolicy, MediaRef, PlannedField, ReferencedCommitReport,
    ReferencedSaveError, ReferencedSaveFailure, ReservedStageError, ReservedStagedMedia,
    StagedFile, StagedMedia, UploadPlan, MAX_STAGING_TTL,
};
pub use http::{
    multipart_text_field, read_multipart, read_multipart_repeated, routes, FormPart,
    MultipartError, MULTIPART_BODY_LIMIT,
};
pub use local::{
    detect_type, inspect_image, sanitize_name, BeforeSave, CleanupReport, ImageHeaderCheck,
    ImageInfo, ImageLimits, LocalStorage, StorageError, StoredFile,
};
pub use png::{decode_png, validate_image, ImageValidation};
pub use reconciliation::{
    reconcile_expired, ExpiryCleanup, ReconciliationOutcome, ReferenceCheck, ReferenceDecision,
};
pub use reference_ledger::{
    AbandonedCleanup, AbandonedCleanupItem, CleanupReceipt, ReferenceLedger, ReservedUploadError,
    UploadIntent, UploadObservation, UploadState,
};
pub use s3::{
    credentials_from_env, prepare as prepare_s3, S3Credentials, S3Plan, ACCESS_KEY_ENV,
    SECRET_KEY_ENV,
};
pub use signed::{open_download, sign_download, sign_download_for, MAX_DOWNLOAD_TTL};

use async_trait::async_trait;
use uuid::Uuid;

#[async_trait]
pub trait Storage: Send + Sync {
    async fn save(
        &self,
        owner: Uuid,
        original_name: &str,
        bytes: &[u8],
    ) -> Result<StoredFile, StorageError>;
    async fn read(&self, owner: Uuid, id: Uuid) -> Result<(StoredFile, Vec<u8>), StorageError>;
    async fn delete(&self, owner: Uuid, id: Uuid) -> Result<(), StorageError>;
    async fn exists(&self, owner: Uuid, id: Uuid) -> Result<bool, StorageError>;
    async fn metadata(&self, owner: Uuid, id: Uuid) -> Result<StoredFile, StorageError>;

    /// The original owner retains ownership. Other actors are rejected.
    async fn save_for(
        &self,
        actor: Uuid,
        owner: Uuid,
        original_name: &str,
        bytes: &[u8],
    ) -> Result<StoredFile, StorageError> {
        if actor != owner {
            return Err(StorageError::Forbidden);
        }
        self.save(owner, original_name, bytes).await
    }

    async fn read_for(
        &self,
        actor: Uuid,
        owner: Uuid,
        id: Uuid,
    ) -> Result<(StoredFile, Vec<u8>), StorageError> {
        if actor != owner {
            return Err(StorageError::Forbidden);
        }
        self.read(owner, id).await
    }

    /// Stores bytes that expire unless [`Storage::retain`] clears the deadline.
    ///
    /// The default rejects a deadline rather than silently storing permanent data.
    /// A backend that stages uploads must persist the deadline and honor `retain`.
    async fn save_expiring(
        &self,
        owner: Uuid,
        original_name: &str,
        bytes: &[u8],
        expires_at: Option<i64>,
    ) -> Result<StoredFile, StorageError> {
        if expires_at.is_some() {
            return Err(StorageError::Failed);
        }
        self.save(owner, original_name, bytes).await
    }

    /// Create a staged object at a fresh, server-allocated ID. Never reuse IDs,
    /// including after failure or deletion. Backends must refuse collisions.
    /// This low-level method requires an external admission protocol.
    async fn save_reserved(
        &self,
        media: MediaRef,
        original_name: &str,
        bytes: &[u8],
        expires_at: i64,
    ) -> Result<StoredFile, StorageError> {
        let _ = (media, original_name, bytes, expires_at);
        Err(StorageError::Failed)
    }

    /// Clears an expiry written by [`Storage::save_expiring`].
    ///
    /// The default fails. A database commit does not call this method.
    async fn retain(&self, owner: Uuid, id: Uuid) -> Result<StoredFile, StorageError> {
        let _ = (owner, id);
        Err(StorageError::Failed)
    }
}

#[cfg(test)]
mod contract_tests {
    use super::*;

    struct BasicOnly(LocalStorage);

    #[async_trait]
    impl Storage for BasicOnly {
        async fn save(
            &self,
            owner: Uuid,
            original_name: &str,
            bytes: &[u8],
        ) -> Result<StoredFile, StorageError> {
            self.0.save(owner, original_name, bytes).await
        }

        async fn read(&self, owner: Uuid, id: Uuid) -> Result<(StoredFile, Vec<u8>), StorageError> {
            self.0.read(owner, id).await
        }

        async fn delete(&self, owner: Uuid, id: Uuid) -> Result<(), StorageError> {
            self.0.delete(owner, id).await
        }

        async fn exists(&self, owner: Uuid, id: Uuid) -> Result<bool, StorageError> {
            self.0.exists(owner, id).await
        }

        async fn metadata(&self, owner: Uuid, id: Uuid) -> Result<StoredFile, StorageError> {
            self.0.metadata(owner, id).await
        }
    }

    #[tokio::test]
    async fn basic_backend_rejects_staging_before_writing() {
        let root = std::env::temp_dir().join(format!("fexu-basic-{}", Uuid::new_v4()));
        let storage = BasicOnly(LocalStorage::open(&root, 64, &["text/plain"]).unwrap());
        let owner = Uuid::new_v4();
        assert_eq!(
            storage
                .save_expiring(owner, "note.txt", b"staged", Some(100))
                .await,
            Err(StorageError::Failed)
        );
        assert!(!root.join(owner.to_string()).exists());
        let permanent = storage
            .save_expiring(owner, "note.txt", b"permanent", None)
            .await
            .unwrap();
        assert_eq!(
            storage.read(owner, permanent.id).await.unwrap().1,
            b"permanent"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
