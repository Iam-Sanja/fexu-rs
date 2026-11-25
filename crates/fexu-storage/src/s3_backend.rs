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
use object_store::{ClientOptions, ObjectStore, ObjectStoreExt, PutMode, RetryConfig, UpdateVersion};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const MAGIC: &[u8; 4] = b"FXS1";
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

    async fn put(&self, meta: &StoredFile, bytes: &[u8], mode: PutMode) -> Result<(), StorageError> {
        let payload = encode(meta, bytes)?;
        self.store
            .put_opts(&Self::path(meta.owner, meta.id), payload.into(), mode.into())
            .await
            .map_err(map_error)?;
        Ok(())
    }

    async fn load(
        &self,
        owner: Uuid,
        id: Uuid,
    ) -> Result<(StoredFile, Vec<u8>, UpdateVersion), StorageError> {
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
        let (meta, bytes) = decode(&body, owner, id, self.limits.max_bytes)?;
        Ok((meta, bytes.to_vec(), version))
    }
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
