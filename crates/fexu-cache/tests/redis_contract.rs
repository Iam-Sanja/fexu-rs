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

async fn connections_received(observer: &mut redis::aio::MultiplexedConnection) -> u64 {
    let stats: String = redis::cmd("INFO")
        .arg("stats")
        .query_async(observer)
        .await
        .unwrap();
    stats
        .lines()
        .find_map(|line| line.strip_prefix("total_connections_received:"))
        .expect("connection counter")
        .trim()
        .parse()
        .unwrap()
}

#[tokio::test]
#[ignore = "requires isolated Redis fixture"]
async fn redis_connection_reuse_contract() {
    let (url, prefix) = fixture();
    let mut observer = redis::Client::open(url.as_str())
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let before = connections_received(&mut observer).await;
    let backend = RedisCache::new(&url, &prefix, RedisLimits::default()).unwrap();
    let mut tasks = Vec::new();
    for index in 0..16 {
        let cache = TypedCache::new(backend.clone(), 1);
        tasks.push(tokio::spawn(async move {
            let namespace = format!("worker-{index}");
            cache.set(&namespace, "key", &index, None).await.unwrap();
            assert_eq!(
                cache.get::<i32>(&namespace, "key").await.unwrap(),
                Some(index)
            );
            cache.invalidate_namespace(&namespace).await.unwrap();
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    assert_eq!(connections_received(&mut observer).await, before + 1);
    // A separate backend has its own connection, but sees the same data.
    let first = TypedCache::new(backend, 1);
    let second = TypedCache::new(
        RedisCache::new(&url, &prefix, RedisLimits::default()).unwrap(),
        1,
    );
    first.set("shared", "key", &"value", None).await.unwrap();
    assert_eq!(
        second
            .get::<String>("shared", "key")
            .await
            .unwrap()
            .as_deref(),
        Some("value")
    );
    assert_eq!(connections_received(&mut observer).await, before + 2);
}

async fn wait_for_marker(root: &std::path::Path, name: &str) {
    tokio::time::timeout(Duration::from_secs(60), async {
        while !root.join(name).is_file() {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("fixture coordination timed out");
}

#[tokio::test]
#[ignore = "requires the isolated Redis restart shell fixture"]
async fn redis_restart_contract() {
    let (url, prefix) = fixture();
    let root = std::path::PathBuf::from(
        std::env::var_os("FEXU_TEST_REDIS_MARKERS").expect("fixture markers"),
    );
    let backend = RedisCache::new(&url, &prefix, RedisLimits::default()).unwrap();
    let first = TypedCache::new(backend.clone(), 1);
    let second = TypedCache::new(
        RedisCache::new(&url, &prefix, RedisLimits::default()).unwrap(),
        1,
    );
    first.set("app", "old", &"before", None).await.unwrap();
    assert_eq!(
        second
            .get::<String>("app", "old")
            .await
            .unwrap()
            .as_deref(),
        Some("before")
    );
    let token = first
        .backend()
        .current_token("app", "old")
        .await
        .unwrap();
    std::fs::write(root.join("ready"), b"ready").unwrap();
    wait_for_marker(&root, "stopped").await;
    assert_eq!(
        first.get::<String>("app", "old").await,
        Err(CacheError::Backend)
    );
    assert_eq!(
        second.set("app", "old", &"failed", None).await,
        Err(CacheError::Backend)
    );
    std::fs::write(root.join("observed"), b"observed").unwrap();
    wait_for_marker(&root, "restarted").await;
    // Keep the original handles alive across the whole outage.
    assert_eq!(first.get::<String>("app", "old").await.unwrap(), None);
    second.set("app", "new", &"after", None).await.unwrap();
    assert_eq!(
        first
            .backend()
            .put_raw_if_current("app", "old", b"stale".to_vec(), None, token)
            .await
            .unwrap(),
        Publish::Superseded
    );
    assert_eq!(first.get::<String>("app", "old").await.unwrap(), None);
    assert_eq!(
        first
            .get::<String>("app", "new")
            .await
            .unwrap()
            .as_deref(),
        Some("after")
    );
}

#[tokio::test]
#[ignore = "requires isolated TLS Redis fixture"]
async fn redis_tls_contract() {
    let url = std::env::var("FEXU_TEST_REDIS_TLS_URL").expect("TLS fixture URL");
    assert!(url.starts_with("rediss://127.0.0.1:"));
    let prefix = format!("test-{}", Uuid::new_v4().simple());
    let cache = TypedCache::new(
        RedisCache::new(&url, &prefix, RedisLimits::default()).unwrap(),
        1,
    );
    let trusted = std::env::var("FEXU_TEST_REDIS_TLS_TRUSTED").unwrap();
    if trusted == "yes" {
        cache.set("app", "key", &"tls", None).await.unwrap();
        assert_eq!(
            cache.get::<String>("app", "key").await.unwrap().as_deref(),
            Some("tls")
        );
    } else {
        assert_eq!(trusted, "no");
        assert_eq!(
            cache.get::<String>("app", "key").await,
            Err(CacheError::Backend)
        );
        assert_eq!(
            cache.set("app", "key", &"no", None).await,
            Err(CacheError::Backend)
        );
    }
}

#[test]
fn redis_configuration_rejects_insecure_urls_and_unbounded_timeout() {
    for url in [
        "http://127.0.0.1:6379/",
        "rediss://127.0.0.1:6379/#insecure",
        "rediss://127.0.0.1:6379/?insecure=true",
    ] {
        assert!(matches!(
            RedisCache::new(url, "app", RedisLimits::default()),
            Err(CacheError::Backend)
        ));
    }
    assert!(matches!(
        RedisCache::new(
            "redis://127.0.0.1:6379/",
            "app",
            RedisLimits {
                timeout: Duration::from_secs(61),
                ..RedisLimits::default()
            }
        ),
        Err(CacheError::InvalidLimits)
    ));
}
