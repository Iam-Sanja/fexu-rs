//! Begrenzter Prozessspeicher. Er ist kein gemeinsamer Cache mehrerer Instanzen.

use crate::backend::{
    validate_key, validate_namespace, CacheBackend, Publish, RawLookup, WriteToken,
};
use crate::error::CacheError;
use async_trait::async_trait;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryLimits {
    pub max_entries: usize,
    pub max_bytes: usize,
}

impl Default for MemoryLimits {
    fn default() -> Self {
        Self {
            max_entries: 1024,
            max_bytes: 8 * 1024 * 1024,
        }
    }
}

#[derive(Clone)]
pub struct MemoryCache {
    inner: Arc<MemoryInner>,
}

struct MemoryInner {
    state: Mutex<MemoryState>,
    now_ms: Arc<dyn Fn() -> i64 + Send + Sync>,
}

struct MemoryState {
    entries: HashMap<(String, String), Slot>,
    bytes: usize,
    limits: MemoryLimits,
    epochs: Epochs,
}

/// Prozesslokale Marken. Vergessene Marken heben `floor` an, damit ein alter
/// Token eher abgelehnt als nach einer Löschung noch akzeptiert wird.
struct Epochs {
    clock: u64,
    floor: u64,
    namespaces: HashMap<String, u64>,
    namespace_order: VecDeque<(String, u64)>,
    keys: HashMap<(String, String), u64>,
    key_order: VecDeque<((String, String), u64)>,
    cap: usize,
}

struct Slot {
    bytes: Vec<u8>,
    stored_at_ms: i64,
    expires_at_ms: Option<i64>,
}

impl MemoryCache {
    pub fn new(limits: MemoryLimits) -> Result<Self, CacheError> {
        Self::with_now(
            limits,
            Arc::new(|| {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_millis() as i64)
                    .unwrap_or(0)
            }),
        )
    }

    pub fn with_now(
        limits: MemoryLimits,
        now_ms: Arc<dyn Fn() -> i64 + Send + Sync>,
    ) -> Result<Self, CacheError> {
        if limits.max_entries == 0 || limits.max_bytes == 0 {
            return Err(CacheError::InvalidLimits);
        }
        Ok(Self {
            inner: Arc::new(MemoryInner {
                state: Mutex::new(MemoryState {
                    entries: HashMap::new(),
                    bytes: 0,
                    epochs: Epochs::new(limits.max_entries.saturating_mul(8).max(64)),
                    limits,
                }),
                now_ms,
            }),
        })
    }

    fn now(&self) -> i64 {
        (self.inner.now_ms)()
    }
}

#[async_trait]
impl CacheBackend for MemoryCache {
    async fn lookup_raw(&self, namespace: &str, key: &str) -> Result<RawLookup, CacheError> {
        validate_namespace(namespace)?;
        validate_key(key)?;
        let mut state = lock(&self.inner.state);
        let map_key = (namespace.to_owned(), key.to_owned());
        let expired = state
            .entries
            .get(&map_key)
            .is_some_and(|slot| expired(slot, self.now()));
        if expired {
            remove_key(&mut state, &map_key);
            return Ok(RawLookup::Expired);
        }
        match state.entries.get(&map_key) {
            Some(slot) => Ok(RawLookup::Hit(slot.bytes.clone())),
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
        validate_namespace(namespace)?;
        validate_key(key)?;
        if ttl.is_some_and(|ttl| ttl.is_zero()) {
            return Err(CacheError::InvalidTtl);
        }
        let mut state = lock(&self.inner.state);
        let now = self.now();
        let map_key = (namespace.to_owned(), key.to_owned());
        store_locked(&mut state, map_key.clone(), bytes, ttl, now)?;
        state.epochs.note_key(&map_key.0, &map_key.1);
        Ok(())
    }

    async fn current_token(&self, namespace: &str, key: &str) -> Result<WriteToken, CacheError> {
        validate_namespace(namespace)?;
        validate_key(key)?;
        let state = lock(&self.inner.state);
        Ok(state.epochs.observe())
    }

    async fn put_raw_if_current(
        &self,
        namespace: &str,
        key: &str,
        bytes: Vec<u8>,
        ttl: Option<Duration>,
        token: WriteToken,
    ) -> Result<Publish, CacheError> {
        validate_namespace(namespace)?;
        validate_key(key)?;
        if ttl.is_some_and(|ttl| ttl.is_zero()) {
            return Err(CacheError::InvalidTtl);
        }
        let mut state = lock(&self.inner.state);
        if !state.epochs.allows(namespace, key, token) {
            return Ok(Publish::Superseded);
        }
        let now = self.now();
        let map_key = (namespace.to_owned(), key.to_owned());
        store_locked(&mut state, map_key, bytes, ttl, now)?;
        Ok(Publish::Stored)
    }

    async fn delete_raw(&self, namespace: &str, key: &str) -> Result<(), CacheError> {
        validate_namespace(namespace)?;
        validate_key(key)?;
        let mut state = lock(&self.inner.state);
        state.epochs.note_key(namespace, key);
        remove_key(&mut state, &(namespace.to_owned(), key.to_owned()));
        Ok(())
    }

    async fn invalidate_namespace(&self, namespace: &str) -> Result<(), CacheError> {
        validate_namespace(namespace)?;
        let mut state = lock(&self.inner.state);
        state.epochs.note_namespace(namespace);
        let keys: Vec<_> = state
            .entries
            .keys()
            .filter(|(entry_namespace, _)| entry_namespace == namespace)
            .cloned()
            .collect();
        for key in keys {
            remove_key(&mut state, &key);
        }
        Ok(())
    }
}

fn store_locked(
    state: &mut MemoryState,
    map_key: (String, String),
    bytes: Vec<u8>,
    ttl: Option<Duration>,
    now: i64,
) -> Result<(), CacheError> {
    if bytes.len() > state.limits.max_bytes {
        return Err(CacheError::TooLarge);
    }
    remove_key(state, &map_key);
    evict_expired(state, now);
    while state.entries.len() >= state.limits.max_entries
        || state.bytes.saturating_add(bytes.len()) > state.limits.max_bytes
    {
        if !evict_one(state) {
            return Err(CacheError::TooLarge);
        }
    }
    let expires_at_ms = ttl.map(|ttl| now.saturating_add(millis(ttl)));
    state.bytes = state.bytes.saturating_add(bytes.len());
    state.entries.insert(
        map_key,
        Slot {
            bytes,
            stored_at_ms: now,
            expires_at_ms,
        },
    );
    Ok(())
}

impl Epochs {
    fn new(cap: usize) -> Self {
        Self {
            clock: 0,
            floor: 0,
            namespaces: HashMap::new(),
            namespace_order: VecDeque::new(),
            keys: HashMap::new(),
            key_order: VecDeque::new(),
            cap,
        }
    }

    fn observe(&self) -> WriteToken {
        WriteToken {
            observed: self.clock,
        }
    }

    fn allows(&self, namespace: &str, key: &str, token: WriteToken) -> bool {
        // u64::MAX ist kein unterscheidbarer Stand mehr. Dann lieber kein Eintrag.
        if token.observed == u64::MAX || token.observed < self.floor {
            return false;
        }
        let namespace_epoch = self.namespaces.get(namespace).copied().unwrap_or(0);
        let key_epoch = self
            .keys
            .get(&(namespace.to_owned(), key.to_owned()))
            .copied()
            .unwrap_or(0);
        namespace_epoch <= token.observed && key_epoch <= token.observed
    }

    fn note_namespace(&mut self, namespace: &str) {
        let epoch = self.bump();
        self.namespaces.insert(namespace.to_owned(), epoch);
        self.namespace_order
            .push_back((namespace.to_owned(), epoch));
        self.prune();
    }

    fn note_key(&mut self, namespace: &str, key: &str) {
        let epoch = self.bump();
        let map_key = (namespace.to_owned(), key.to_owned());
        self.keys.insert(map_key.clone(), epoch);
        self.key_order.push_back((map_key, epoch));
        self.prune();
    }

    fn bump(&mut self) -> u64 {
        self.clock = self.clock.saturating_add(1);
        self.clock
    }

    fn prune(&mut self) {
        while self.key_order.len() > self.cap {
            let Some((key, epoch)) = self.key_order.pop_front() else {
                break;
            };
            if self.keys.get(&key).copied() == Some(epoch) {
                self.keys.remove(&key);
                self.floor = self.floor.max(epoch);
            }
        }
        while self.namespace_order.len() > self.cap {
            let Some((namespace, epoch)) = self.namespace_order.pop_front() else {
                break;
            };
            if self.namespaces.get(&namespace).copied() == Some(epoch) {
                self.namespaces.remove(&namespace);
                self.floor = self.floor.max(epoch);
            }
        }
    }
}

fn lock(mutex: &Mutex<MemoryState>) -> std::sync::MutexGuard<'_, MemoryState> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn expired(slot: &Slot, now: i64) -> bool {
    slot.expires_at_ms.is_some_and(|expires| expires <= now)
}

fn evict_expired(state: &mut MemoryState, now: i64) {
    let keys: Vec<_> = state
        .entries
        .iter()
        .filter(|(_, slot)| expired(slot, now))
        .map(|(key, _)| key.clone())
        .collect();
    for key in keys {
        remove_key(state, &key);
    }
}

fn evict_one(state: &mut MemoryState) -> bool {
    let Some(key) = state
        .entries
        .iter()
        .min_by(|(left_key, left), (right_key, right)| {
            (
                left.expires_at_ms.unwrap_or(i64::MAX),
                left.stored_at_ms,
                left_key,
            )
                .cmp(&(
                    right.expires_at_ms.unwrap_or(i64::MAX),
                    right.stored_at_ms,
                    right_key,
                ))
        })
        .map(|(key, _)| key.clone())
    else {
        return false;
    };
    remove_key(state, &key);
    true
}

fn remove_key(state: &mut MemoryState, key: &(String, String)) {
    if let Some(slot) = state.entries.remove(key) {
        state.bytes = state.bytes.saturating_sub(slot.bytes.len());
    }
}

fn millis(ttl: Duration) -> i64 {
    i64::try_from(ttl.as_millis()).unwrap_or(i64::MAX).max(1)
}
