#![cfg(feature = "redis-cache")]

use fexu_cache::{
    CacheBackend, CacheError, CacheLookup, Publish, RedisCache, RedisLimits, TypedCache,
};
use std::time::Duration;
use uuid::Uuid;

fn fixture() -> (String, String) {
    let url = std::env::var("FEXU_TEST_REDIS_URL").expect("disposable Redis URL");
    assert!(url.starts_with("redis://127.0.0.1:"));
    (url, format!("test-{}", Uuid::new_v4().simple()))
}

#[tokio::test]
#[ignore = "requires isolated Redis fixture"]
async fn redis_shared_contract() {
    let (url, prefix) = fixture();
    let limits = RedisLimits {
        max_value_bytes: 256,
        default_ttl: Duration::from_secs(30),
        ..RedisLimits::default()
    };
    let first = TypedCache::new(RedisCache::new(&url, &prefix, limits).unwrap(), 1);
    let second = TypedCache::new(RedisCache::new(&url, &prefix, limits).unwrap(), 1);
    let computed: String = first
        .get_or_set("tenant", "computed", None, || async {
            Ok("computed".to_owned())
        })
        .await
        .unwrap();
    assert_eq!(computed, "computed");
    assert_eq!(
        second
            .get::<String>("tenant", "computed")
            .await
            .unwrap()
            .as_deref(),
        Some("computed")
    );
    let pending = first.get_or_set("tenant", "pending", None, || async {
        second.invalidate_namespace("tenant").await?;
        Ok("stale".to_owned())
    });
    assert_eq!(pending.await, Err(CacheError::Superseded));
    assert!(second
        .get::<String>("tenant", "pending")
        .await
        .unwrap()
        .is_none());
    first.set("tenant", "key", &"old", None).await.unwrap();
    first.set("tenant.a", "key", &"other", None).await.unwrap();
    assert_eq!(
        second
            .get::<String>("tenant", "key")
            .await
            .unwrap()
            .as_deref(),
        Some("old")
    );
    let token = first
        .backend()
        .current_token("tenant", "key")
        .await
        .unwrap();
    second.set("tenant", "key", &"new", None).await.unwrap();
    assert_eq!(
        first
            .backend()
            .put_raw_if_current("tenant", "key", b"stale".to_vec(), None, token)
            .await
            .unwrap(),
        Publish::Superseded
    );
    assert_eq!(
        first
            .get::<String>("tenant", "key")
            .await
            .unwrap()
            .as_deref(),
        Some("new")
    );
    let token = first
        .backend()
        .current_token("tenant", "key")
        .await
        .unwrap();
    second.delete("tenant", "key").await.unwrap();
    assert_eq!(
        first
            .backend()
            .put_raw_if_current("tenant", "key", b"stale".to_vec(), None, token)
            .await
            .unwrap(),
        Publish::Superseded
    );
    let token = first
        .backend()
        .current_token("tenant", "key")
        .await
        .unwrap();
    second.invalidate_namespace("tenant").await.unwrap();
    second.set("tenant", "fresh", &"fresh", None).await.unwrap();
    assert_eq!(
        first
            .backend()
            .put_raw_if_current("tenant", "key", b"stale".to_vec(), None, token)
            .await
            .unwrap(),
        Publish::Superseded
    );
    assert_eq!(
        first.lookup::<String>("tenant", "key").await.unwrap(),
        CacheLookup::Miss
    );
    assert_eq!(
        first
            .get::<String>("tenant.a", "key")
            .await
            .unwrap()
            .as_deref(),
        Some("other")
    );
    let other = TypedCache::new(
        RedisCache::new(&url, &format!("{prefix}-other"), limits).unwrap(),
        1,
    );
    assert!(other
        .get::<String>("tenant.a", "key")
        .await
        .unwrap()
        .is_none());
    first
        .set(
            "tenant",
            "expiry",
            &"short",
            Some(Duration::from_millis(50)),
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        second.lookup::<String>("tenant", "expiry").await.unwrap(),
        CacheLookup::Miss
    );
    assert_eq!(
        first.set("tenant", "fresh", &"x".repeat(300), None).await,
        Err(CacheError::TooLarge)
    );
    assert_eq!(
        second
            .get::<String>("tenant", "fresh")
            .await
            .unwrap()
            .as_deref(),
        Some("fresh")
    );
    assert_eq!(
        first
            .set("tenant", "zero", &"x", Some(Duration::ZERO))
            .await,
        Err(CacheError::InvalidTtl)
    );
    let incompatible = TypedCache::new(RedisCache::new(&url, &prefix, limits).unwrap(), 2);
    assert_eq!(
        incompatible.get::<String>("tenant", "fresh").await,
        Err(CacheError::VersionMismatch)
    );
    first.invalidate_namespace("tenant").await.unwrap();
    first.invalidate_namespace("tenant.a").await.unwrap();
}

#[tokio::test]
#[ignore = "requires stopped isolated Redis fixture"]
async fn redis_outage_contract() {
    let (url, prefix) = fixture();
    let cache = TypedCache::new(
        RedisCache::new(&url, &prefix, RedisLimits::default()).unwrap(),
        1,
    );
    assert_eq!(
        cache.get::<String>("app", "key").await,
        Err(CacheError::Backend)
    );
    assert_eq!(
        cache.set("app", "key", &"value", None).await,
        Err(CacheError::Backend)
    );
    assert_eq!(
        cache.invalidate_namespace("app").await,
        Err(CacheError::Backend)
    );
}
