//! Private, bounded S3 objects with atomic metadata and payload publication.

use crate::local::{bytes_match, prepare_file};
use crate::{
    prepare_s3, sanitize_name, BeforeSave, S3Credentials, S3Plan, Storage, StorageError, StoredFile,
};
use async_trait::async_trait;
use futures_util::StreamExt;
use object_store::aws::{AmazonS3, AmazonS3Builder, S3ConditionalPut};
use object_store::client::{HttpClient, HttpConnector};
use object_store::path::Path;
use object_store::{
    ClientOptions, ObjectStore, ObjectStoreExt, PutMode, RetryConfig, UpdateVersion,
};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const MAGIC: &[u8; 4] = b"FXS1";
const DELETED_MAGIC: &[u8; 4] = b"FXD1";
const FENCED_MAGIC: &[u8; 4] = b"FXF1";
const MAX_METADATA: usize = 4096;
const MAX_FILE: u64 = 100 * 1024 * 1024;

/// Bounds apply to one object, not to the total size of the bucket.
#[derive(Clone, Copy, Debug)]
pub struct S3Limits {
    pub max_bytes: u64,
    pub timeout: Duration,
}

impl Default for S3Limits {
    fn default() -> Self {
        Self {
            max_bytes: 10 * 1024 * 1024,
            timeout: Duration::from_secs(15),
        }
    }
}

#[derive(Clone)]
pub struct S3Storage {
    store: Arc<AmazonS3>,
    limits: S3Limits,
    allow: Vec<String>,
    check: Option<Arc<dyn BeforeSave>>,
}

// The configured client is the only transport; no implicit proxy or redirect.
#[derive(Debug, Clone)]
struct DirectConnector(reqwest::Client);

impl HttpConnector for DirectConnector {
    fn connect(&self, _options: &ClientOptions) -> object_store::Result<HttpClient> {
        Ok(HttpClient::new(self.0.clone()))
    }
}

impl S3Storage {
    /// Connect to a pre-provisioned private bucket. Construction does not
    /// contact the service. An optional PEM CA bundle supplements system trust.
    pub fn new(
        plan: &S3Plan,
        credentials: S3Credentials,
        limits: S3Limits,
        allow: &[&str],
        ca_pem: Option<&[u8]>,
    ) -> Result<Self, StorageError> {
        let plan = prepare_s3(
            &plan.endpoint,
            &plan.region,
            &plan.bucket,
            &plan.access_key_env,
            &plan.secret_key_env,
        )?;
        if !(1..=MAX_FILE).contains(&limits.max_bytes)
            || limits.timeout.is_zero()
            || limits.timeout > Duration::from_secs(60)
            || allow.is_empty()
            || credentials.access_key.len() < 16
            || credentials.secret_key.len() < 16
            || credentials.access_key.chars().any(char::is_control)
            || credentials.secret_key.chars().any(char::is_control)
        {
            return Err(StorageError::Failed);
        }
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            let _ = rustls::crypto::ring::default_provider().install_default();
        }
        let mut client = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .no_zstd()
            .timeout(limits.timeout)
            .connect_timeout(limits.timeout);
        if let Some(pem) = ca_pem {
            let certs =
                reqwest::Certificate::from_pem_bundle(pem).map_err(|_| StorageError::Failed)?;
            if certs.is_empty() {
                return Err(StorageError::Failed);
            }
            client = client.tls_certs_merge(certs);
        }
        let client = client.build().map_err(|_| StorageError::Failed)?;
        let store = AmazonS3Builder::new()
            .with_endpoint(plan.endpoint)
            .with_region(plan.region)
            .with_bucket_name(plan.bucket)
            .with_access_key_id(credentials.access_key)
            .with_secret_access_key(credentials.secret_key)
            .with_virtual_hosted_style_request(false)
            .with_conditional_put(S3ConditionalPut::ETagMatch)
            .with_retry(RetryConfig {
                max_retries: 0,
                ..RetryConfig::default()
            })
            .with_http_connector(DirectConnector(client))
            .build()
            .map_err(|_| StorageError::Failed)?;
        Ok(Self {
            store: Arc::new(store),
            limits,
            allow: allow.iter().map(|value| (*value).to_owned()).collect(),
            check: None,
        })
    }

    pub fn with_check(mut self, check: Arc<dyn BeforeSave>) -> Self {
        self.check = Some(check);
        self
    }

    fn path(owner: Uuid, id: Uuid) -> Path {
        Path::from(format!("fexu-media/v1/{owner}/{id}"))
    }

    async fn put(
        &self,
        meta: &StoredFile,
        bytes: &[u8],
        mode: PutMode,
    ) -> Result<(), StorageError> {
        let payload = encode(meta, bytes)?;
        self.store
            .put_opts(
                &Self::path(meta.owner, meta.id),
                payload.into(),
                mode.into(),
            )
            .await
            .map_err(map_error)?;
        Ok(())
    }

    async fn load(
        &self,
        owner: Uuid,
        id: Uuid,
    ) -> Result<(StoredFile, Vec<u8>, UpdateVersion), StorageError> {
        let (body, version) = self.load_raw(owner, id).await?;
        if body == tombstone(owner, id) || body == upload_fence(owner, id) {
            return Err(StorageError::NotFound);
        }
        let (meta, bytes) = decode(&body, owner, id, self.limits.max_bytes)?;
        Ok((meta, bytes.to_vec(), version))
    }

    async fn load_raw(
        &self,
        owner: Uuid,
        id: Uuid,
    ) -> Result<(Vec<u8>, UpdateVersion), StorageError> {
        let result = self
            .store
            .get(&Self::path(owner, id))
            .await
            .map_err(map_error)?;
        let limit = self.limits.max_bytes + MAX_METADATA as u64 + 8;
        if result.meta.size > limit {
            return Err(StorageError::TooLarge);
        }
        let expected_len = result.meta.size;
        let version = UpdateVersion {
            e_tag: result.meta.e_tag.clone(),
            version: result.meta.version.clone(),
        };
        let mut body = Vec::new();
        let mut stream = result.into_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(map_error)?;
            if body.len() as u64 + chunk.len() as u64 > limit {
                return Err(StorageError::TooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        if body.len() as u64 != expected_len {
            return Err(StorageError::Failed);
        }
        Ok((body, version))
    }

    // Only used after validating the full expired envelope; keep the observed
    // version across the conditional write rather than fetching a newer ETag.
    async fn mark_expired(
        &self,
        owner: Uuid,
        id: Uuid,
        version: UpdateVersion,
    ) -> Result<(), StorageError> {
        if version.e_tag.is_none() {
            return Err(StorageError::Failed);
        }
        self.store
            .put_opts(
                &Self::path(owner, id),
                tombstone(owner, id).into(),
                PutMode::Update(version).into(),
            )
            .await
            .map_err(map_error)?;
        Ok(())
    }

    // Called only after the ledger durably blocks the staging transition.
    // A permanent marker occupies the exact conditional-create key.
    pub(crate) async fn fence_upload(&self, media: crate::MediaRef) -> Result<(), StorageError> {
        let marker = upload_fence(media.owner, media.id);
        let mode = match self.load_raw(media.owner, media.id).await {
            Ok((body, version)) => {
                if body == marker {
                    return Ok(());
                }
                if body != tombstone(media.owner, media.id) {
                    decode(&body, media.owner, media.id, self.limits.max_bytes)?;
                }
                if version.e_tag.is_none() {
                    return Err(StorageError::Failed);
                }
                PutMode::Update(version)
            }
            Err(StorageError::NotFound) => PutMode::Create,
            Err(error) => return Err(error),
        };
        self.store
            .put_opts(
                &Self::path(media.owner, media.id),
                marker.into(),
                mode.into(),
            )
            .await
            .map_err(map_error)?;
        Ok(())
    }

    // Read only: a missing marker must not be recreated by quota release.
    pub(crate) async fn verify_upload_fence(
        &self,
        media: crate::MediaRef,
    ) -> Result<(), StorageError> {
        let (body, _) = self.load_raw(media.owner, media.id).await?;
        if body != upload_fence(media.owner, media.id) {
            return Err(StorageError::Failed);
        }
        Ok(())
    }

    /// Remove an expired object that the application has established is unreferenced.
    ///
    /// This is an operator-level operation, not an authorization or database
    /// reference check. The caller must exclude new references while reconciling.
    /// A conditional replacement fences concurrent retain before physical removal.
    /// A failed removal can be retried with the same owner and immutable file ID.
    pub async fn cleanup_unreferenced_expired(
        &self,
        owner: Uuid,
        id: Uuid,
    ) -> Result<bool, StorageError> {
        let (body, version) = match self.load_raw(owner, id).await {
            Ok(value) => value,
            Err(StorageError::NotFound) => return Ok(false),
            Err(error) => return Err(error),
        };
        if body == upload_fence(owner, id) {
            return Ok(false);
        }
        let marker = tombstone(owner, id);
        let path = Self::path(owner, id);
        if body != marker {
            let (meta, _) = decode(&body, owner, id, self.limits.max_bytes)?;
            if !expired(&meta) {
                return Ok(false);
            }
            self.mark_expired(owner, id, version).await?;
        }
        // No public operation can retain or recreate this immutable key after
        // the marker is committed. Never delete if the conditional PUT failed.
        match self.store.delete(&path).await {
            Ok(()) => Ok(true),
            Err(object_store::Error::NotFound { .. }) => Ok(false),
            Err(error) => Err(map_error(error)),
        }
    }
}

#[async_trait]
impl crate::ExpiryCleanup for S3Storage {
    async fn cleanup_expired(&self, media: crate::MediaRef) -> Result<bool, StorageError> {
        self.cleanup_unreferenced_expired(media.owner, media.id)
            .await
    }
}

fn tombstone(owner: Uuid, id: Uuid) -> Vec<u8> {
    let mut body = Vec::with_capacity(36);
    body.extend_from_slice(DELETED_MAGIC);
    body.extend_from_slice(owner.as_bytes());
    body.extend_from_slice(id.as_bytes());
    body
}

fn upload_fence(owner: Uuid, id: Uuid) -> Vec<u8> {
    let mut body = Vec::with_capacity(36);
    body.extend_from_slice(FENCED_MAGIC);
    body.extend_from_slice(owner.as_bytes());
    body.extend_from_slice(id.as_bytes());
    body
}

fn map_error(error: object_store::Error) -> StorageError {
    match error {
        object_store::Error::NotFound { .. } => StorageError::NotFound,
        _ => StorageError::Failed,
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn expired(meta: &StoredFile) -> bool {
    meta.expires_at.is_some_and(|deadline| deadline <= now())
}

fn encode(meta: &StoredFile, bytes: &[u8]) -> Result<Vec<u8>, StorageError> {
    let json = serde_json::to_vec(meta).map_err(|_| StorageError::Failed)?;
    if json.len() > MAX_METADATA {
        return Err(StorageError::Failed);
    }
    let mut payload = Vec::with_capacity(8 + json.len() + bytes.len());
    payload.extend_from_slice(MAGIC);
    payload.extend_from_slice(&(json.len() as u32).to_be_bytes());
    payload.extend_from_slice(&json);
    payload.extend_from_slice(bytes);
    Ok(payload)
}

fn decode(
    body: &[u8],
    owner: Uuid,
    id: Uuid,
    max_bytes: u64,
) -> Result<(StoredFile, &[u8]), StorageError> {
    if body.len() < 8 || &body[..4] != MAGIC {
        return Err(StorageError::Failed);
    }
    let len = u32::from_be_bytes(body[4..8].try_into().map_err(|_| StorageError::Failed)?) as usize;
    if len > MAX_METADATA || body.len() < 8 + len {
        return Err(StorageError::Failed);
    }
    let meta: StoredFile =
        serde_json::from_slice(&body[8..8 + len]).map_err(|_| StorageError::Failed)?;
    let bytes = &body[8 + len..];
    if meta.owner != owner || meta.id != id {
        return Err(StorageError::NotFound);
    }
    if bytes.len() as u64 > max_bytes {
        return Err(StorageError::TooLarge);
    }
    if meta.original_name != sanitize_name(&meta.original_name)
        || meta.sha256.len() != 64
        || !bytes_match(&meta, bytes)
        || crate::detect_type(bytes) != Some(meta.content_type.as_str())
    {
        return Err(StorageError::Failed);
    }
    Ok((meta, bytes))
}

#[async_trait]
impl Storage for S3Storage {
    async fn save(
        &self,
        owner: Uuid,
        original_name: &str,
        bytes: &[u8],
    ) -> Result<StoredFile, StorageError> {
        self.save_expiring(owner, original_name, bytes, None).await
    }

    async fn save_expiring(
        &self,
        owner: Uuid,
        original_name: &str,
        bytes: &[u8],
        expires_at: Option<i64>,
    ) -> Result<StoredFile, StorageError> {
        let meta = prepare_file(
            owner,
            original_name,
            bytes,
            expires_at,
            self.limits.max_bytes,
            &self.allow,
            self.check.as_deref(),
        )?;
        self.put(&meta, bytes, PutMode::Create).await?;
        Ok(meta)
    }

    async fn save_reserved(
        &self,
        media: crate::MediaRef,
        original_name: &str,
        bytes: &[u8],
        expires_at: i64,
    ) -> Result<StoredFile, StorageError> {
        let mut meta = prepare_file(
            media.owner,
            original_name,
            bytes,
            Some(expires_at),
            self.limits.max_bytes,
            &self.allow,
            self.check.as_deref(),
        )?;
        meta.id = media.id;
        self.put(&meta, bytes, PutMode::Create).await?;
        Ok(meta)
    }

    async fn read(&self, owner: Uuid, id: Uuid) -> Result<(StoredFile, Vec<u8>), StorageError> {
        let (meta, bytes, _) = self.load(owner, id).await?;
        if expired(&meta) {
            return Err(StorageError::Expired);
        }
        Ok((meta, bytes))
    }

    async fn delete(&self, owner: Uuid, id: Uuid) -> Result<(), StorageError> {
        // Check identity even for an expired object. A competing retain can
        // never resurrect a deleted key because it uses If-Match.
        self.load(owner, id).await?;
        self.store
            .delete(&Self::path(owner, id))
            .await
            .map_err(map_error)
    }

    async fn exists(&self, owner: Uuid, id: Uuid) -> Result<bool, StorageError> {
        match self.metadata(owner, id).await {
            Ok(meta) => Ok(!expired(&meta)),
            Err(StorageError::NotFound) => Ok(false),
            Err(error) => Err(error),
        }
    }

    async fn metadata(&self, owner: Uuid, id: Uuid) -> Result<StoredFile, StorageError> {
        self.load(owner, id).await.map(|(meta, _, _)| meta)
    }

    async fn retain(&self, owner: Uuid, id: Uuid) -> Result<StoredFile, StorageError> {
        let (mut meta, bytes, version) = self.load(owner, id).await?;
        if expired(&meta) {
            return Err(StorageError::Expired);
        }
        if meta.expires_at.is_none() {
            return Ok(meta);
        }
        if version.e_tag.is_none() {
            return Err(StorageError::Failed);
        }
        meta.expires_at = None;
        self.put(&meta, &bytes, PutMode::Update(version)).await?;
        Ok(meta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Private access deliberately permits deterministic interleavings without
    // exposing raw writes or failure injection in the public API.
    fn recovery_fixture() -> S3Storage {
        let endpoint = std::env::var("FEXU_TEST_S3_ENDPOINT").unwrap();
        assert!(endpoint.starts_with("https://127.0.0.1:"));
        let plan = prepare_s3(
            &endpoint,
            "us-east-1",
            "fexu-test-media",
            crate::ACCESS_KEY_ENV,
            crate::SECRET_KEY_ENV,
        )
        .unwrap();
        let ca = std::fs::read(std::env::var_os("FEXU_TEST_S3_CA").unwrap()).unwrap();
        S3Storage::new(
            &plan,
            crate::credentials_from_env(&plan).unwrap(),
            S3Limits::default(),
            &["text/plain"],
            Some(&ca),
        )
        .unwrap()
    }

    #[tokio::test]
    #[ignore = "requires isolated TLS S3 fixture"]
    async fn s3_cleanup_permanent_fence_contract() {
        let first = recovery_fixture();
        let second = recovery_fixture();
        for present in [false, true] {
            let media = crate::MediaRef {
                owner: Uuid::new_v4(),
                id: Uuid::new_v4(),
            };
            if present {
                first
                    .save_reserved(media, "note.txt", b"original", now() + 300)
                    .await
                    .unwrap();
            }
            first.fence_upload(media).await.unwrap();
            second.fence_upload(media).await.unwrap();
            assert!(second
                .save_reserved(media, "note.txt", b"late", now() + 300)
                .await
                .is_err());
            assert_eq!(
                second.read(media.owner, media.id).await,
                Err(StorageError::NotFound)
            );
            assert_eq!(
                second.delete(media.owner, media.id).await,
                Err(StorageError::NotFound)
            );
            assert!(!second
                .cleanup_unreferenced_expired(media.owner, media.id)
                .await
                .unwrap());
            assert_eq!(
                second.load_raw(media.owner, media.id).await.unwrap().0,
                upload_fence(media.owner, media.id)
            );
        }
    }

    #[tokio::test]
    #[ignore = "requires isolated TLS S3 fixture"]
    async fn s3_cleanup_ledger_fence_contract() {
        let first = recovery_fixture();
        let second = recovery_fixture();
        let db = fexu_db::connect("sqlite::memory:").await.unwrap();
        let ledger = crate::ReferenceLedger::new(db.clone());
        ledger.initialize().await.unwrap();
        let owner = Uuid::new_v4();
        let failed = ledger
            .stage_reserved(&first, owner, "note.exe", b"pending", now() + 300)
            .await
            .unwrap_err();
        let media = failed.media;
        assert_eq!(failed.error, StorageError::TypeRejected);
        assert!(!ledger.upload_fence_requested(media).await.unwrap());
        assert_eq!(
            ledger.upload_state(media).await.unwrap(),
            Some(crate::UploadState::Uploading)
        );
        assert!(!ledger
            .release_fenced_s3_quota(&first, media)
            .await
            .unwrap());
        ledger.abandon_unresolved_s3(&first, media).await.unwrap();
        assert!(ledger.upload_fence_requested(media).await.unwrap());
        assert_eq!(
            ledger.upload_state(media).await.unwrap(),
            Some(crate::UploadState::Abandoned)
        );
        let reopened = crate::ReferenceLedger::new(db.clone());
        reopened
            .abandon_unresolved_s3(&second, media)
            .await
            .unwrap();
        let page = reopened
            .cleanup_abandoned_page(&second, None, 10)
            .await
            .unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].media, media);
        assert_eq!(page[0].outcome, Ok(crate::AbandonedCleanup::Missing));
        assert_eq!(
            reopened.cleanup_receipt(media).await.unwrap(),
            Some(crate::CleanupReceipt::Missing)
        );
        assert!(first
            .save_reserved(media, "note.txt", b"late", now() + 300)
            .await
            .is_err());
        assert_eq!(
            second.load_raw(media.owner, media.id).await.unwrap().0,
            upload_fence(media.owner, media.id)
        );
        let quota = reopened.owner_quota(owner).await.unwrap().unwrap();
        assert_eq!(quota.reserved_files, 1);
        assert_eq!(quota.reserved_bytes, 7);
        // Simulate an externally removed marker: a receipt is insufficient.
        second
            .store
            .delete(&S3Storage::path(owner, media.id))
            .await
            .unwrap();
        assert!(reopened
            .release_fenced_s3_quota(&second, media)
            .await
            .is_err());
        let quota = reopened.owner_quota(owner).await.unwrap().unwrap();
        assert_eq!(quota.reserved_files, 1);
        reopened
            .abandon_unresolved_s3(&second, media)
            .await
            .unwrap();
        let (left, right) = tokio::join!(
            ledger.release_fenced_s3_quota(&first, media),
            reopened.release_fenced_s3_quota(&second, media)
        );
        assert_ne!(left.unwrap(), right.unwrap());
        assert!(!reopened
            .release_fenced_s3_quota(&second, media)
            .await
            .unwrap());
        let quota = reopened.owner_quota(owner).await.unwrap().unwrap();
        assert_eq!(quota.reserved_files, 0);
        assert_eq!(quota.reserved_bytes, 0);
        assert_eq!(
            second.load_raw(owner, media.id).await.unwrap().0,
            upload_fence(owner, media.id)
        );
        db.close().await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires isolated TLS S3 fixture"]
    async fn s3_cleanup_reserved_collision_contract() {
        let first = recovery_fixture();
        let second = recovery_fixture();
        let media = crate::MediaRef {
            owner: Uuid::new_v4(),
            id: Uuid::new_v4(),
        };
        let stored = first
            .save_reserved(media, "note.txt", b"original", now() + 300)
            .await
            .unwrap();
        assert_eq!(stored.id, media.id);
        assert!(second
            .save_reserved(media, "note.txt", b"overwrite", now() + 300)
            .await
            .is_err());
        assert_eq!(
            first.read(media.owner, media.id).await.unwrap().1,
            b"original"
        );
        first.delete(media.owner, media.id).await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires isolated TLS S3 fixture"]
    async fn s3_cleanup_stale_etag_contract() {
        let first = recovery_fixture();
        let second = recovery_fixture();
        let owner = Uuid::new_v4();
        let staged = first
            .save_expiring(owner, "stale.txt", b"keep", Some(now() + 300))
            .await
            .unwrap();
        let (_, _, stale) = first.load(owner, staged.id).await.unwrap();
        let retained = second.retain(owner, staged.id).await.unwrap();

        // Model a previously observed version arriving after the retain.
        // The private marker write must fail, regardless of caller timing.
        assert_eq!(
            first.mark_expired(owner, staged.id, stale).await,
            Err(StorageError::Failed)
        );
        assert_eq!(
            second.read(owner, staged.id).await.unwrap(),
            (retained, b"keep".to_vec())
        );
        assert!(!first
            .cleanup_unreferenced_expired(owner, staged.id)
            .await
            .unwrap());
        second.delete(owner, staged.id).await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires isolated TLS S3 fixture"]
    async fn s3_cleanup_interrupted_marker_contract() {
        let first = recovery_fixture();
        let second = recovery_fixture();
        let owner = Uuid::new_v4();
        let staged = first
            .save_expiring(owner, "interrupted.txt", b"discard", Some(now() - 1))
            .await
            .unwrap();
        let (_, _, stale) = first.load(owner, staged.id).await.unwrap();
        let (_, _, version) = second.load(owner, staged.id).await.unwrap();
        second
            .mark_expired(owner, staged.id, version)
            .await
            .unwrap();

        // Stop after the marker, before physical deletion. A stale write
        // cannot replace it and a fresh client can finish reconciliation.
        let mut retained = staged.clone();
        retained.expires_at = None;
        assert_eq!(
            first
                .put(&retained, b"discard", PutMode::Update(stale))
                .await,
            Err(StorageError::Failed)
        );
        assert_eq!(
            first.read(owner, staged.id).await,
            Err(StorageError::NotFound)
        );
        assert_eq!(
            first.metadata(owner, staged.id).await,
            Err(StorageError::NotFound)
        );
        assert_eq!(
            first.retain(owner, staged.id).await,
            Err(StorageError::NotFound)
        );
        assert!(!first.exists(owner, staged.id).await.unwrap());
        assert!(first
            .cleanup_unreferenced_expired(owner, staged.id)
            .await
            .unwrap());
        assert!(matches!(
            second.load_raw(owner, staged.id).await,
            Err(StorageError::NotFound)
        ));
        assert!(!second
            .cleanup_unreferenced_expired(owner, staged.id)
            .await
            .unwrap());
    }

    #[test]
    fn envelope_rejects_corruption_truncation_and_wrong_identity() {
        let owner = Uuid::new_v4();
        let meta = prepare_file(
            owner,
            "../note.txt",
            b"hello",
            None,
            64,
            &["text/plain".to_owned()],
            None,
        )
        .unwrap();
        let body = encode(&meta, b"hello").unwrap();
        let (decoded, bytes) = decode(&body, owner, meta.id, 64).unwrap();
        assert_eq!(decoded, meta);
        assert_eq!(bytes, b"hello");
        assert_eq!(decoded.original_name, "note.txt");
        assert_eq!(
            decode(&body, Uuid::new_v4(), meta.id, 64),
            Err(StorageError::NotFound)
        );
        assert_eq!(
            decode(&body, owner, meta.id, 4),
            Err(StorageError::TooLarge)
        );
        assert_eq!(
            decode(&body[..body.len() - 1], owner, meta.id, 64),
            Err(StorageError::Failed)
        );
        let mut corrupt = body;
        *corrupt.last_mut().unwrap() = b'x';
        assert_eq!(
            decode(&corrupt, owner, meta.id, 64),
            Err(StorageError::Failed)
        );
    }
}
