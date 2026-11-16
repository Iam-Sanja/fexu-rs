//! Typed values with a version prefix. Mismatched prefixes are errors.

use crate::backend::{validate_key, validate_namespace, CacheBackend, Publish, RawLookup};
use crate::error::CacheError;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;

const MAGIC: &[u8; 4] = b"FXC1";

/// Ergebnis einer typisierten Suche.
///
/// `get` bildet `Miss` und `Expired` auf `None` ab. `lookup` unterscheidet sie.
/// Expired entries are removed on first read; subsequent reads return `Miss`.
/// A future backend that deletes expired keys may report `Expired` as `Miss`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheLookup<T> {
    Hit(T),
    Miss,
    Expired,
}

pub struct TypedCache<B> {
    backend: B,
    version: u32,
    flights: Mutex<HashMap<String, Arc<Flight>>>,
}

struct Flight {
    result: Mutex<Option<Result<Vec<u8>, CacheError>>>,
    notify: Notify,
    waiters: AtomicU32,
}

impl<B> TypedCache<B> {
    pub fn new(backend: B, version: u32) -> Self {
        Self {
            backend,
            version,
            flights: Mutex::new(HashMap::new()),
        }
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    #[cfg(test)]
    pub(crate) fn parked_waiters(&self) -> u32 {
        let guard = self
            .flights
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard
            .values()
            .map(|flight| flight.waiters.load(Ordering::SeqCst))
            .sum()
    }
}

impl<B: CacheBackend> TypedCache<B> {
    pub async fn lookup<T: DeserializeOwned>(
        &self,
        namespace: &str,
        key: &str,
    ) -> Result<CacheLookup<T>, CacheError> {
        match self.backend.lookup_raw(namespace, key).await? {
            RawLookup::Miss => Ok(CacheLookup::Miss),
            RawLookup::Expired => Ok(CacheLookup::Expired),
            RawLookup::Hit(bytes) => decode::<T>(&bytes, self.version).map(CacheLookup::Hit),
        }
    }

    pub async fn get<T: DeserializeOwned>(
        &self,
        namespace: &str,
        key: &str,
    ) -> Result<Option<T>, CacheError> {
        Ok(match self.lookup(namespace, key).await? {
            CacheLookup::Hit(value) => Some(value),
            CacheLookup::Miss | CacheLookup::Expired => None,
        })
    }

    pub async fn set<T: Serialize>(
        &self,
        namespace: &str,
        key: &str,
        value: &T,
        ttl: Option<Duration>,
    ) -> Result<(), CacheError> {
        let payload = serde_json::to_vec(value).map_err(|_| CacheError::Serialization)?;
        self.backend
            .put_raw(namespace, key, encode(self.version, &payload), ttl)
            .await
    }

    pub async fn delete(&self, namespace: &str, key: &str) -> Result<(), CacheError> {
        self.backend.delete_raw(namespace, key).await
    }

    pub async fn invalidate_namespace(&self, namespace: &str) -> Result<(), CacheError> {
        self.backend.invalidate_namespace(namespace).await
    }

    /// Gleichzeitige Aufrufe auf diesem `TypedCache` warten auf den ersten Produzenten.
    /// Ein zweiter `TypedCache`, auch auf demselben Speicher, und ein zweiter Prozess rechnen selbst.
    pub async fn get_or_set<T, F, Fut>(
        &self,
        namespace: &str,
        key: &str,
        ttl: Option<Duration>,
        produce: F,
    ) -> Result<T, CacheError>
    where
        T: Serialize + DeserializeOwned,
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, CacheError>>,
    {
        validate_namespace(namespace)?;
        validate_key(key)?;
        if ttl.is_some_and(|ttl| ttl.is_zero()) {
            return Err(CacheError::InvalidTtl);
        }
        if let Some(value) = self.get(namespace, key).await? {
            return Ok(value);
        }
        let flight_key = format!("{namespace}\u{1f}{key}");
        let (flight, leader) = elect(&self.flights, &flight_key);
        if !leader {
            let bytes = wait_for(flight).await?;
            return decode(&bytes, self.version);
        }
        let mut guard = LeaderGuard {
            flights: &self.flights,
            key: flight_key.clone(),
            flight: flight.clone(),
            armed: true,
        };
        let token = match self.backend.current_token(namespace, key).await {
            Ok(token) => token,
            Err(error) => {
                guard.finish(Err(error.clone()));
                return Err(error);
            }
        };
        let produced = produce().await;
        let value = match produced {
            Ok(value) => value,
            Err(error) => {
                guard.finish(Err(error.clone()));
                return Err(error);
            }
        };
        let payload = match serde_json::to_vec(&value) {
            Ok(payload) => payload,
            Err(_) => {
                guard.finish(Err(CacheError::Serialization));
                return Err(CacheError::Serialization);
            }
        };
        let bytes = encode(self.version, &payload);
        match self
            .backend
            .put_raw_if_current(namespace, key, bytes.clone(), ttl, token)
            .await
        {
            Ok(Publish::Stored) => {
                guard.finish(Ok(bytes));
                Ok(value)
            }
            Ok(Publish::Superseded) => {
                guard.finish(Err(CacheError::Superseded));
                Err(CacheError::Superseded)
            }
            Err(error) => {
                guard.finish(Err(error.clone()));
                Err(error)
            }
        }
    }
}

struct LeaderGuard<'a> {
    flights: &'a Mutex<HashMap<String, Arc<Flight>>>,
    key: String,
    flight: Arc<Flight>,
    armed: bool,
}

impl LeaderGuard<'_> {
    fn finish(&mut self, result: Result<Vec<u8>, CacheError>) {
        if self.armed {
            self.armed = false;
            finish(self.flights, &self.key, &self.flight, result);
        }
    }
}

impl Drop for LeaderGuard<'_> {
    fn drop(&mut self) {
        self.finish(Err(CacheError::Backend));
    }
}

fn elect(flights: &Mutex<HashMap<String, Arc<Flight>>>, key: &str) -> (Arc<Flight>, bool) {
    let mut guard = flights
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(existing) = guard.get(key) {
        return (existing.clone(), false);
    }
    let flight = Arc::new(Flight {
        result: Mutex::new(None),
        notify: Notify::new(),
        waiters: AtomicU32::new(0),
    });
    guard.insert(key.to_owned(), flight.clone());
    (flight, true)
}

fn finish(
    flights: &Mutex<HashMap<String, Arc<Flight>>>,
    key: &str,
    flight: &Flight,
    result: Result<Vec<u8>, CacheError>,
) {
    {
        let mut slot = flight
            .result
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *slot = Some(result);
    }
    {
        let mut guard = flights
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.remove(key);
    }
    flight.notify.notify_waiters();
}

async fn wait_for(flight: Arc<Flight>) -> Result<Vec<u8>, CacheError> {
    loop {
        // Subscribe before reading the slot. `notify_waiters` does not store a
        // permit, so a producer that finishes in that gap would leave the waiter
        // parked forever.
        let notified = flight.notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if let Some(result) = flight
            .result
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
        {
            return result;
        }
        flight.waiters.fetch_add(1, Ordering::SeqCst);
        let _parked = WaiterGuard(&flight.waiters);
        notified.await;
    }
}

struct WaiterGuard<'a>(&'a AtomicU32);

impl Drop for WaiterGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn encode(version: u32, payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(8 + payload.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&version.to_be_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

fn decode<T: DeserializeOwned>(bytes: &[u8], version: u32) -> Result<T, CacheError> {
    if bytes.len() < 8 || &bytes[0..4] != MAGIC {
        return Err(CacheError::Corrupt);
    }
    let stored = u32::from_be_bytes(bytes[4..8].try_into().map_err(|_| CacheError::Corrupt)?);
    if stored != version {
        return Err(CacheError::VersionMismatch);
    }
    serde_json::from_slice(&bytes[8..]).map_err(|_| CacheError::Corrupt)
}
