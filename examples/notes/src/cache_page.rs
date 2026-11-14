//! Expliziter Cache für die Notiz-App.
//!
//! `notes.list` bleibt ohne Cache. Aufrufer nutzen [`NoteCache`] ausdrücklich.
//! `MemoryCache::new` gilt nur in diesem Prozess. Ein zweites `NoteCache` hat
//! einen eigenen Speicher. Innerhalb eines `NoteCache` teilen sich Zusammenfassung
//! und Seite dasselbe Limit. Der Stampede-Schutz hängt an jedem einzelnen `TypedCache`.

use std::future::Future;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use fexu::cache::{
    CacheError, CacheLookup, MemoryCache, MemoryLimits, TypedCache, ViewCache, ViewScope,
};

pub struct NoteCache {
    records: TypedCache<MemoryCache>,
    pages: ViewCache<MemoryCache>,
    renders: AtomicU32,
}

impl NoteCache {
    /// `records` und `pages` teilen ein Speicherlimit. Eine zweite Instanz tut das nicht.
    pub fn new(max_entries: usize, max_bytes: usize) -> Result<Self, CacheError> {
        let memory = MemoryCache::new(MemoryLimits {
            max_entries,
            max_bytes,
        })?;
        let records = TypedCache::new(memory.clone(), 1);
        let pages = ViewCache::new(TypedCache::new(memory, 1), "notes")?;
        Ok(Self {
            records,
            pages,
            renders: AtomicU32::new(0),
        })
    }

    /// Mandanten bleiben in dem Namensraum, den der Aufrufer nennt.
    /// Keine Passwörter, Sitzungstoken oder TOTP-Geheimnisse ablegen.
    pub async fn remember_summary(&self, tenant: &str, summary: &str) -> Result<(), CacheError> {
        self.records
            .set(tenant, "summary", &summary, Some(Duration::from_secs(60)))
            .await
    }

    pub async fn summary(&self, tenant: &str) -> Result<Option<String>, CacheError> {
        self.records.get(tenant, "summary").await
    }

    pub async fn summary_state(&self, tenant: &str) -> Result<CacheLookup<String>, CacheError> {
        self.records.lookup(tenant, "summary").await
    }

    pub async fn invalidate_tenant(&self, tenant: &str) -> Result<(), CacheError> {
        self.records.invalidate_namespace(tenant).await
    }

    pub async fn public_list(&self, viewer_is_authenticated: bool) -> Result<String, CacheError> {
        self.render_page(
            "list",
            &ViewScope::Anonymous,
            viewer_is_authenticated,
            || async { Ok(public_list_markup()) },
        )
        .await
    }

    pub async fn user_list<F, Fut>(&self, user_id: &str, produce: F) -> Result<String, CacheError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<String, CacheError>>,
    {
        self.render_page(
            "list",
            &ViewScope::User {
                user_id: user_id.to_owned(),
            },
            true,
            produce,
        )
        .await
    }

    pub async fn invalidate_public_list(&self) -> Result<(), CacheError> {
        self.pages.invalidate("list", &ViewScope::Anonymous).await
    }

    pub fn page_renders(&self) -> u32 {
        self.renders.load(Ordering::SeqCst)
    }

    async fn render_page<F, Fut>(
        &self,
        key: &str,
        scope: &ViewScope,
        viewer_is_authenticated: bool,
        produce: F,
    ) -> Result<String, CacheError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<String, CacheError>>,
    {
        let renders = &self.renders;
        self.pages
            .render(
                key,
                Duration::from_secs(30),
                scope,
                viewer_is_authenticated,
                || async move {
                    renders.fetch_add(1, Ordering::SeqCst);
                    produce().await
                },
            )
            .await
    }
}

pub fn public_list_markup() -> String {
    "<ul><li>notes</li></ul>".to_owned()
}
