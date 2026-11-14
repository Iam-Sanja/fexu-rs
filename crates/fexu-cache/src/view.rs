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
/// `User` speichert unter einer kodierten Nutzer-ID. Die Anwendung muss die ID
/// der angemeldeten Sitzung übergeben. Der Cache prüft keine Sitzung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewScope {
    Anonymous,
    User { user_id: String },
}

/// Opt-in-Cache für fertiges HTML. Tera und `views::render` rufen ihn nicht auf.
///
/// Anonyme Einträge liegen in `view.{name}`, nutzerbezogene in `view.{name}.user`.
/// `invalidate_anonymous` löscht die zweite Gruppe nicht. Ein gemeinsamer Eintrag
/// wird nie für einen angemeldeten Betrachter geschrieben oder ausgeliefert.
pub struct ViewCache<B> {
    cache: TypedCache<B>,
    anonymous: String,
    users: String,
}

impl<B: CacheBackend> ViewCache<B> {
    pub fn new(cache: TypedCache<B>, view: &str) -> Result<Self, CacheError> {
        let anonymous = view_namespace(view, "")?;
        let users = view_namespace(view, "user")?;
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

fn view_namespace(view: &str, suffix: &str) -> Result<String, CacheError> {
    validate_namespace(view)?;
    let namespace = if suffix.is_empty() {
        format!("view.{view}")
    } else {
        format!("view.{view}.{suffix}")
    };
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
