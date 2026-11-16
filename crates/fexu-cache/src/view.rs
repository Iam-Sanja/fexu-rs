//! Explicit fragment and view cache. It does not hook into the template system.

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

/// Who may see a rendered view or template.
///
/// `Anonymous` is shared and rejects authenticated viewers.
/// `User` stores data under an encoded user ID and rejects anonymous viewers
/// . The application must supply the actual session and matching user ID
/// . `viewer_is_authenticated` is not proof of authentication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewScope {
    Anonymous,
    User { user_id: String },
}

/// Opt-in cache for rendered HTML. Tera and `views::render` do not invoke it.
///
/// Anonymous entries use `a{length}.{name}`; user entries use `u{length}.{name}`.
/// Encoding the length distinguishes namespaces `notes` and `notes.user`.
/// `invalidate_anonymous` does not delete user entries. Shared entries
/// are never stored for authenticated viewers. A user scope without
/// an authenticated viewer is rejected before reads. The boolean is not
/// proof of a session: applications must verify sessions and user IDs themselves.
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
    // Reserve four characters for the prefix, length, and dot. Limit names to 60
    // so namespaces remain within the 64-character limit.
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
