//! Opt-in standalone Redis cache. All mutations and fences are atomic on Redis.

use crate::{
    validate_key, validate_namespace, CacheBackend, CacheError, Publish, RawLookup, WriteToken,
};
use async_trait::async_trait;
use redis::aio::MultiplexedConnection;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::{timeout_at, Instant};
use uuid::Uuid;

const MAX_TTL_MS: u64 = 86_400_000;
const META_TTL_MS: u64 = MAX_TTL_MS + 60_000;
const MAX_TIMEOUT: Duration = Duration::from_secs(60);

// KEYS[1] is namespace metadata; ARGV[1] is its data-key prefix.
// Metadata is generation (32 hex chars) + fence (16 hex chars).
// Missing metadata never resurrects old data or accepts an old token.
// Namespace-wide fences deliberately supersede unrelated producers within
// that namespace, avoiding unbounded per-key tombstones.
const SCRIPT: &str = r#"
local state = redis.call('GET', KEYS[1])
local prefix, op, key, generation, fence, ttl, bytes, expected =
    ARGV[1], ARGV[2], ARGV[3], ARGV[4], ARGV[5], ARGV[6], ARGV[7], ARGV[8]
if state and (#state ~= 48 or not string.match(state, '^%x+$')) then
    return redis.error_reply('invalid cache metadata')
end
if op == 'invalidate' then
    redis.call('DEL', KEYS[1])
    return '1'
end
if op == 'lookup' then
    if not state then return false end
    return redis.call('GET', prefix .. string.sub(state, 1, 32) .. ':' .. key)
end
if op == 'publish' and (not state or string.sub(state, 33) ~= expected) then
    return '0'
end
if op == 'delete' and not state then return '1' end
if state then
    generation = string.sub(state, 1, 32)
end
if op == 'token' then
    if state then return string.sub(state, 33) end
    redis.call('SET', KEYS[1], generation .. fence, 'PX', ARGV[9])
    return fence
end
-- Fence first: if a later data write fails, old producers still cannot publish.
redis.call('SET', KEYS[1], generation .. fence, 'PX', ARGV[9])
local data = prefix .. generation .. ':' .. key
if op == 'delete' then
    redis.call('DEL', data)
else
    redis.call('SET', data, bytes, 'PX', ttl)
end
return '1'
"#;

/// Limits for a shared, disposable Redis cache. Every value has a finite TTL.
#[derive(Clone, Copy, Debug)]
pub struct RedisLimits {
    pub max_value_bytes: usize,
    pub default_ttl: Duration,
    pub timeout: Duration,
}

impl Default for RedisLimits {
    fn default() -> Self {
        Self {
            max_value_bytes: 1024 * 1024,
            default_ttl: Duration::from_secs(300),
            timeout: Duration::from_secs(2),
        }
    }
}

/// Standalone Redis backend. No network call is made by the constructor.
/// Clones share a lazily opened multiplexed connection. Failed operations are
/// not replayed; a subsequent operation can establish a new connection.
/// Use one application-specific prefix across instances. Redis is not a
/// source of truth; all values can disappear at any time.
#[derive(Clone)]
pub struct RedisCache {
    client: redis::Client,
    connections: Arc<Connections>,
    prefix: String,
    limits: RedisLimits,
}

// A short synchronous lock protects only the shared pointer, never I/O.
#[derive(Default)]
struct Connections {
    cached: Mutex<Option<Arc<MultiplexedConnection>>>,
    connecting: tokio::sync::Mutex<()>,
}

impl Connections {
    fn cached(&self) -> Result<Option<Arc<MultiplexedConnection>>, CacheError> {
        self.cached
            .lock()
            .map(|guard| guard.clone())
            .map_err(|_| CacheError::Backend)
    }

    fn discard(&self, failed: &Arc<MultiplexedConnection>) {
        if let Ok(mut guard) = self.cached.lock() {
            // A late error from an old connection must not remove a replacement.
            let matches = guard
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, failed));
            if matches {
                *guard = None;
            }
        }
    }

    async fn get(&self, client: &redis::Client) -> Result<Arc<MultiplexedConnection>, CacheError> {
        if let Some(connection) = self.cached()? {
            return Ok(connection);
        }
        let _connecting = self.connecting.lock().await;
        if let Some(connection) = self.cached()? {
            return Ok(connection);
        }
        let connection = Arc::new(
            client
                .get_multiplexed_async_connection()
                .await
                .map_err(|_| CacheError::Backend)?,
        );
        *self.cached.lock().map_err(|_| CacheError::Backend)? = Some(connection.clone());
        Ok(connection)
    }
}

impl RedisCache {
    pub fn new(url: &str, prefix: &str, limits: RedisLimits) -> Result<Self, CacheError> {
        validate_namespace(prefix)?;
        if limits.max_value_bytes == 0 || limits.timeout.is_zero() || limits.timeout > MAX_TIMEOUT {
            return Err(CacheError::InvalidLimits);
        }
        ttl_ms(limits.default_ttl)?;
        if !(url.starts_with("redis://") || url.starts_with("rediss://"))
            || url.contains('#')
            || url.contains('?')
        {
            return Err(CacheError::Backend);
        }
        let client = redis::Client::open(url).map_err(|_| CacheError::Backend)?;
        if url.starts_with("rediss://") && rustls::crypto::CryptoProvider::get_default().is_none() {
            // Keep the application's provider if already installed. Concurrent
            // initializers may race; the first successful installation wins.
            let _ = rustls::crypto::ring::default_provider().install_default();
        }
        Ok(Self {
            client,
            connections: Arc::new(Connections::default()),
            prefix: format!("fexu-cache:v1:{prefix}:"),
            limits,
        })
    }

    fn ttl(&self, ttl: Option<Duration>) -> Result<u64, CacheError> {
        ttl_ms(ttl.unwrap_or(self.limits.default_ttl))
    }

    fn check_bytes(&self, bytes: &[u8]) -> Result<(), CacheError> {
        if bytes.len() > self.limits.max_value_bytes {
            return Err(CacheError::TooLarge);
        }
        Ok(())
    }

    async fn execute(
        &self,
        namespace: &str,
        key: &str,
        op: &str,
        bytes: &[u8],
        ttl: u64,
        expected: u64,
    ) -> Result<Option<Vec<u8>>, CacheError> {
        validate_namespace(namespace)?;
        if op != "invalidate" {
            validate_key(key)?;
        }
        let namespace = format!("{}{namespace}:", self.prefix);
        let generation = Uuid::new_v4().simple().to_string();
        let random = Uuid::new_v4().as_u128();
        let fence = format!("{:016x}", (random >> 64) as u64 ^ random as u64);
        let deadline = Instant::now() + self.limits.timeout;
        let shared = timeout_at(deadline, self.connections.get(&self.client))
            .await
            .map_err(|_| CacheError::Backend)??;
        let mut connection = shared.as_ref().clone();
        let mut command = redis::cmd("EVAL");
        command
            .arg(SCRIPT)
            .arg(1)
            .arg(format!("{namespace}meta"))
            .arg(format!("{namespace}data:"))
            .arg(op)
            .arg(key)
            .arg(generation)
            .arg(fence)
            .arg(ttl)
            .arg(bytes)
            .arg(format!("{expected:016x}"))
            .arg(META_TTL_MS);
        let result = timeout_at(
            deadline,
            command.query_async::<Option<Vec<u8>>>(&mut connection),
        )
        .await;
        match result {
            Ok(Ok(result)) => Ok(result),
            _ => {
                // The server might have executed the command even if the caller
                // did not receive its reply. Never replay it automatically.
                self.connections.discard(&shared);
                Err(CacheError::Backend)
            }
        }
    }
}

fn ttl_ms(ttl: Duration) -> Result<u64, CacheError> {
    if ttl.is_zero() || ttl > Duration::from_millis(MAX_TTL_MS) {
        return Err(CacheError::InvalidTtl);
    }
    u64::try_from(ttl.as_nanos().div_ceil(1_000_000)).map_err(|_| CacheError::InvalidTtl)
}

fn stored(result: Option<Vec<u8>>) -> Result<(), CacheError> {
    if result.as_deref() == Some(b"1") {
        Ok(())
    } else {
        Err(CacheError::Backend)
    }
}

#[async_trait]
impl CacheBackend for RedisCache {
    async fn lookup_raw(&self, namespace: &str, key: &str) -> Result<RawLookup, CacheError> {
        match self.execute(namespace, key, "lookup", &[], 0, 0).await? {
            Some(bytes) => {
                self.check_bytes(&bytes)?;
                Ok(RawLookup::Hit(bytes))
            }
            None => Ok(RawLookup::Miss),
        }
    }

    async fn put_raw(
        &self,
        namespace: &str,
        key: &str,
        bytes: Vec<u8>,
        ttl: Option<Duration>,
    ) -> Result<(), CacheError> {
        self.check_bytes(&bytes)?;
        stored(
            self.execute(namespace, key, "set", &bytes, self.ttl(ttl)?, 0)
                .await?,
        )
    }

    async fn delete_raw(&self, namespace: &str, key: &str) -> Result<(), CacheError> {
        stored(self.execute(namespace, key, "delete", &[], 0, 0).await?)
    }

    async fn invalidate_namespace(&self, namespace: &str) -> Result<(), CacheError> {
        stored(self.execute(namespace, "", "invalidate", &[], 0, 0).await?)
    }

    async fn current_token(&self, namespace: &str, key: &str) -> Result<WriteToken, CacheError> {
        let bytes = self
            .execute(namespace, key, "token", &[], 0, 0)
            .await?
            .ok_or(CacheError::Backend)?;
        let text = std::str::from_utf8(&bytes).map_err(|_| CacheError::Backend)?;
        let observed = u64::from_str_radix(text, 16).map_err(|_| CacheError::Backend)?;
        Ok(WriteToken { observed })
    }

    async fn put_raw_if_current(
        &self,
        namespace: &str,
        key: &str,
        bytes: Vec<u8>,
        ttl: Option<Duration>,
        token: WriteToken,
    ) -> Result<Publish, CacheError> {
        self.check_bytes(&bytes)?;
        let result = self
            .execute(
                namespace,
                key,
                "publish",
                &bytes,
                self.ttl(ttl)?,
                token.observed,
            )
            .await?;
        match result.as_deref() {
            Some(b"1") => Ok(Publish::Stored),
            Some(b"0") => Ok(Publish::Superseded),
            _ => Err(CacheError::Backend),
        }
    }
}
