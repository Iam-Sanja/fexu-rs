//! Common interface for in-process caches and future external storage.
//!
//! No Redis or other network-backed driver is included.
//! [`crate::MemoryCache`] is scoped to the process that created it. Clones share
//! entries; separately constructed caches and other processes do not.
//!
//! Cache keys consist of a namespace and a key. Implementations must not
//! concatenate them using a delimiter permitted within either part.
//! [`validate_namespace`] and [`validate_key`] define the shared validation rules.
//! `invalidate_namespace` matches only the exact namespace, not longer prefixes.

use crate::error::CacheError;
use async_trait::async_trait;
use std::time::Duration;

/// Raw bytes without type checking.
///
/// `Expired` assumes the backend still tracks expiration. A future
/// Redis backend may report expired keys as `Miss` if the server has
/// already deleted them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawLookup {
    Hit(Vec<u8>),
    Miss,
    Expired,
}

/// Observed state of deletions and invalidations.
///
/// This value is only a token. Backends must validate and write it under
/// the same exclusion lock used by `delete_raw` and `invalidate_namespace`.
/// Two separate calls are insufficient: invalidation can occur between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteToken {
    pub observed: u64,
}

/// Result of a conditional publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Publish {
    Stored,
    /// The state in [`WriteToken`] is obsolete. Nothing was written.
    Superseded,
}

#[async_trait]
pub trait CacheBackend: Send + Sync {
    async fn lookup_raw(&self, namespace: &str, key: &str) -> Result<RawLookup, CacheError>;

    async fn get_raw(&self, namespace: &str, key: &str) -> Result<Option<Vec<u8>>, CacheError> {
        Ok(match self.lookup_raw(namespace, key).await? {
            RawLookup::Hit(bytes) => Some(bytes),
            RawLookup::Miss | RawLookup::Expired => None,
        })
    }

    async fn put_raw(
        &self,
        namespace: &str,
        key: &str,
        bytes: Vec<u8>,
        ttl: Option<Duration>,
    ) -> Result<(), CacheError>;

    async fn delete_raw(&self, namespace: &str, key: &str) -> Result<(), CacheError>;

    async fn invalidate_namespace(&self, namespace: &str) -> Result<(), CacheError>;

    /// State token for a later [`Self::put_raw_if_current`] check.
    ///
    /// `MemoryCache` reads it under the entry mutex. Other backends
    /// must do the same under their own deletion lock. The trait cannot enforce
    /// this atomicity.
    async fn current_token(&self, namespace: &str, key: &str) -> Result<WriteToken, CacheError>;

    /// Writes only if `token` still matches the key.
    ///
    /// Validation and writing use the same exclusion lock as `delete_raw`
    /// and `invalidate_namespace`. Setting the same key, calling `delete_raw`,
    /// or calling `invalidate_namespace` invalidates older tokens. Backends
    /// must not implement this method as two disconnected calls.
    async fn put_raw_if_current(
        &self,
        namespace: &str,
        key: &str,
        bytes: Vec<u8>,
        ttl: Option<Duration>,
        token: WriteToken,
    ) -> Result<Publish, CacheError>;
}

/// Namespaces have 1–64 ASCII characters: start alphanumeric, then allow `_`, `-`, `.`.
pub fn validate_namespace(namespace: &str) -> Result<(), CacheError> {
    let mut chars = namespace.chars();
    let ok = matches!(chars.next(), Some(char) if char.is_ascii_alphanumeric())
        && chars
            .all(|char| char.is_ascii_alphanumeric() || char == '_' || char == '-' || char == '.')
        && (1..=64).contains(&namespace.len());
    if ok {
        Ok(())
    } else {
        Err(CacheError::InvalidKey)
    }
}

/// Keys have 1–180 ASCII characters, excluding whitespace, `\\`, and `..`.
pub fn validate_key(key: &str) -> Result<(), CacheError> {
    let ok = !key.is_empty()
        && key.len() <= 180
        && !key.contains("..")
        && key
            .chars()
            .all(|char| char.is_ascii_graphic() && char != ' ' && char != '\\');
    if ok {
        Ok(())
    } else {
        Err(CacheError::InvalidKey)
    }
}
