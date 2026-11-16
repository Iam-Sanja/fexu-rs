//! Cache-layer errors. Messages contain no stored values.

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CacheError {
    #[error("cache key is invalid")]
    InvalidKey,
    #[error("cache value exceeds the memory limit")]
    TooLarge,
    #[error("cache value could not be serialized")]
    Serialization,
    #[error("cached value does not match the expected version")]
    VersionMismatch,
    #[error("cached value is corrupt")]
    Corrupt,
    #[error("cache backend failed")]
    Backend,
    #[error("cache limits must be greater than zero")]
    InvalidLimits,
    #[error("cache ttl must be greater than zero")]
    InvalidTtl,
    #[error("authenticated output is not stored in a shared cache")]
    AuthenticatedContent,
    #[error("user-scoped cache requires an authenticated viewer")]
    UnauthenticatedViewer,
    #[error("cache write was superseded by a delete or invalidation")]
    Superseded,
}
