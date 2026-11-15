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

/// Beobachteter Stand von Löschungen und Invalidierungen.
///
/// Der Wert ist nur eine Marke. Ein Backend muss ihn unter demselben Ausschluss
/// prüfen und schreiben, den `delete_raw` und `invalidate_namespace` benutzen.
/// Zwei getrennte Aufrufe reichen nicht: dazwischen kann eine Invalidierung liegen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteToken {
    pub observed: u64,
}

/// Ergebnis einer bedingten Veröffentlichung.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Publish {
    Stored,
    /// Der Stand in [`WriteToken`] ist überholt. Es wurde nichts geschrieben.
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

    /// Stand für eine spätere [`Self::put_raw_if_current`]-Prüfung.
    ///
    /// `MemoryCache` liest ihn unter dem Mutex der Einträge. Ein anderes Backend
    /// muss dasselbe für seine eigene Löschsperre tun. Das Trait kann diese
    /// Atomarität nicht erzwingen.
    async fn current_token(&self, namespace: &str, key: &str) -> Result<WriteToken, CacheError>;

    /// Schreibt nur, wenn `token` noch zum Schlüssel passt.
    ///
    /// Prüfung und Schreiben laufen unter demselben Ausschluss wie `delete_raw`
    /// und `invalidate_namespace`. Ein `set` desselben Schlüssels, `delete_raw`
    /// und `invalidate_namespace` machen einen älteren Token ungültig. Ein Backend
    /// ohne diese Kopplung darf die Methode nicht als zwei lose Aufrufe nachbauen.
    async fn put_raw_if_current(
        &self,
        namespace: &str,
        key: &str,
        bytes: Vec<u8>,
        ttl: Option<Duration>,
        token: WriteToken,
    ) -> Result<Publish, CacheError>;
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
