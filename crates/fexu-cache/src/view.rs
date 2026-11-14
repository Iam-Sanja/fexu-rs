//! Expliziter Fragment- und View-Cache. Er hängt sich nicht an das Template-System.

use crate::backend::{validate_key, validate_namespace, CacheBackend};
use crate::error::CacheError;
use crate::typed::TypedCache;
use std::future::Future;
use std::time::Duration;

pub struct FragmentCache<B> {
    cache: TypedCache<B>,
    namespace: String,
}

impl<B: CacheBackend> FragmentCache<B> {
    pub fn new(cache: TypedCache<B>, namespace: &str) -> Result<Self, CacheError> {
        let namespace = format!("view.{namespace}");
        validate_namespace(&namespace)?;
        Ok(Self { cache, namespace })
    }

    pub async fn render<F, Fut>(
        &self,
        key: &str,
        ttl: Duration,
        produce: F,
    ) -> Result<String, CacheError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<String, CacheError>>,
    {
        self.cache
            .get_or_set(&self.namespace, key, Some(ttl), produce)
            .await
    }

    pub async fn invalidate(&self, key: &str) -> Result<(), CacheError> {
        self.cache.delete(&self.namespace, key).await
    }

    pub async fn invalidate_all(&self) -> Result<(), CacheError> {
        self.cache.invalidate_namespace(&self.namespace).await
    }
}

/// Wer eine gerenderte View oder ein Template sehen darf.
///
/// `Anonymous` ist ein gemeinsamer Eintrag und lehnt angemeldete Betrachter ab.
/// `User` speichert unter einer kodierten Nutzer-ID und lehnt anonyme Betrachter
/// ab. Die Anwendung muss die echte Sitzung und die dazu passende Nutzer-ID
/// übergeben. `viewer_is_authenticated` ist kein Authentifizierungsnachweis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewScope {
    Anonymous,
    User { user_id: String },
}

/// Opt-in-Cache für fertiges HTML. Tera und `views::render` rufen ihn nicht auf.
///
/// Anonyme Einträge liegen in `a{länge}.{name}`, nutzerbezogene in `u{länge}.{name}`.
/// Die Länge macht `notes` und `notes.user` zu verschiedenen Namensräumen.
/// `invalidate_anonymous` löscht keine Nutzereinträge. Ein gemeinsamer Eintrag
/// wird nie für einen angemeldeten Betrachter geschrieben. Ein Nutzer-Scope ohne
/// angemeldeten Betrachter wird vor dem Lesen abgelehnt. Das Boolean ist kein
/// Nachweis der Sitzung: die Anwendung muss Sitzung und Nutzer-ID selbst prüfen.
pub struct ViewCache<B> {
    cache: TypedCache<B>,
    anonymous: String,
    users: String,
}

impl<B: CacheBackend> ViewCache<B> {
    pub fn new(cache: TypedCache<B>, view: &str) -> Result<Self, CacheError> {
        let anonymous = scope_namespace(view, 'a')?;
        let users = scope_namespace(view, 'u')?;
        Ok(Self {
            cache,
            anonymous,
            users,
        })
    }

    pub async fn render<F, Fut>(
        &self,
        key: &str,
        ttl: Duration,
        scope: &ViewScope,
        viewer_is_authenticated: bool,
        produce: F,
    ) -> Result<String, CacheError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<String, CacheError>>,
    {
        if matches!(scope, ViewScope::Anonymous) && viewer_is_authenticated {
            return Err(CacheError::AuthenticatedContent);
        }
        if matches!(scope, ViewScope::User { .. }) && !viewer_is_authenticated {
            return Err(CacheError::UnauthenticatedViewer);
        }
        let (namespace, storage_key) = self.location(key, scope)?;
        self.cache
            .get_or_set(&namespace, &storage_key, Some(ttl), produce)
            .await
    }

    pub async fn invalidate(&self, key: &str, scope: &ViewScope) -> Result<(), CacheError> {
        let (namespace, storage_key) = self.location(key, scope)?;
        self.cache.delete(&namespace, &storage_key).await
    }

    pub async fn invalidate_anonymous(&self) -> Result<(), CacheError> {
        self.cache.invalidate_namespace(&self.anonymous).await
    }

    pub async fn invalidate_users(&self) -> Result<(), CacheError> {
        self.cache.invalidate_namespace(&self.users).await
    }

    fn location(&self, key: &str, scope: &ViewScope) -> Result<(String, String), CacheError> {
        match scope {
            ViewScope::Anonymous => Ok((self.anonymous.clone(), key.to_owned())),
            ViewScope::User { user_id } => {
                Ok((self.users.clone(), user_storage_key(user_id, key)?))
            }
        }
    }
}

fn scope_namespace(view: &str, scope_tag: char) -> Result<String, CacheError> {
    validate_namespace(view)?;
    // Vier Zeichen für Kennung, Länge und Punkt. Der Name bleibt bei höchstens 60,
    // damit der Namensraum die Grenze von 64 Zeichen einhält.
    if view.len() > 60 {
        return Err(CacheError::InvalidKey);
    }
    let namespace = format!("{scope_tag}{:02}.{view}", view.len());
    validate_namespace(&namespace)?;
    Ok(namespace)
}

pub(crate) fn user_storage_key(user_id: &str, key: &str) -> Result<String, CacheError> {
    validate_key(user_id)?;
    validate_key(key)?;
    let composed = format!("u{:03}:{user_id}:{key}", user_id.len());
    validate_key(&composed)?;
    Ok(composed)
}
