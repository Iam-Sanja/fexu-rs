//! Gemeinsame Schnittstelle für Prozessspeicher und einen späteren externen Speicher.
//!
//! Ein Redis- oder anderer Netzwerktreiber ist nicht enthalten. Ein
//! [`crate::MemoryCache`] gilt nur im Prozess, der ihn erzeugt hat. Ein Klon teilt
//! denselben Speicher. Ein zweites `MemoryCache::new` nicht, und ein zweiter Prozess
//! ebenfalls nicht.
//!
//! Schlüssel sind das Paar aus Namensraum und Schlüssel. Implementierungen dürfen die
//! beiden Teile nicht mit einem Zeichen verketten, das in einem gültigen Schlüssel
//! vorkommen kann. [`validate_namespace`] und [`validate_key`] sind die gemeinsame Regel.
//! `invalidate_namespace` trifft nur denselben Namensraum, keine längeren Namen.

use crate::error::CacheError;
use async_trait::async_trait;
use std::time::Duration;

/// Rohbytes ohne Typprüfung.
///
/// `Expired` setzt voraus, dass das Backend den Ablauf noch kennt. Ein späteres
/// Redis-Backend darf abgelaufene Schlüssel als `Miss` melden, wenn der Server sie
/// bereits gelöscht hat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawLookup {
    Hit(Vec<u8>),
    Miss,
    Expired,
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
}

/// Namensräume sind 1 bis 64 ASCII-Zeichen: Buchstabe oder Ziffer, danach auch `_`, `-`, `.`.
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

/// Schlüssel sind 1 bis 180 ASCII-Zeichen ohne Leerzeichen, `\\` oder `..`.
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
