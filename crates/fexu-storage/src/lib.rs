//! Local file storage. Public delivery is disabled by default.
//!
//! [`s3`] validates remote-storage configuration without opening a connection.
//! Malware scanning is not included. Content validation does not replace antivirus scanning
//! and does not decode image pixels.

mod access;
mod fields;
mod http;
mod local;
mod png;
mod s3;
mod signed;

pub use access::{OwnerAccess, StorageAccess, StorageAction};
pub use fields::{
    prepare_media, upload_name_code, AbortReport, BoundMedia, CommitReport, FieldError, FieldSpec,
    IncomingFile, MediaKind, MediaPolicy, MediaRef, PlannedField, StagedFile, StagedMedia,
    UploadPlan, MAX_STAGING_TTL,
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
    /// The default ignores `expires_at` and calls [`Storage::save`]. A backend that
    /// stages uploads has to keep the deadline and honor `retain`.
    async fn save_expiring(
        &self,
        owner: Uuid,
        original_name: &str,
        bytes: &[u8],
        expires_at: Option<i64>,
    ) -> Result<StoredFile, StorageError> {
        let _ = expires_at;
        self.save(owner, original_name, bytes).await
    }

    /// Clears an expiry written by [`Storage::save_expiring`].
    ///
    /// The default fails. A database commit does not call this method.
    async fn retain(&self, owner: Uuid, id: Uuid) -> Result<StoredFile, StorageError> {
        let _ = (owner, id);
        Err(StorageError::Failed)
    }
}
