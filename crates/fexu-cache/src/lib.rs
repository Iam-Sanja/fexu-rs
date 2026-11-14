//! Optionaler, typisierter Cache. Der mitgelieferte Speicher gilt nur im aktuellen Prozess.
//!
//! Ein externes Backend implementiert [`CacheBackend`]. Fexu liefert keinen
//! Redis-Client und behandelt den Prozessspeicher nicht als verteilten Cache.
//! Zwei `MemoryCache::new`-Werte teilen keine Einträge. Ein `clone` schon, aber
//! nur innerhalb dieses Prozesses. [`ViewCache`] speichert Views nur, wenn die
//! Anwendung ihn aufruft, und lehnt angemeldete Betrachter im gemeinsamen Scope ab.

mod backend;
mod error;
mod memory;
mod typed;
mod view;

pub use backend::{validate_key, validate_namespace, CacheBackend, RawLookup};
pub use error::CacheError;
pub use memory::{MemoryCache, MemoryLimits};
pub use typed::{CacheLookup, TypedCache};
pub use view::{FragmentCache, ViewCache, ViewScope};

#[cfg(test)]
mod tests {
    use super::*;
    use serde::ser::Error;
    use serde::{Deserialize, Serialize, Serializer};
    use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct Note {
        label: String,
    }

    fn clock(now: Arc<AtomicU32>) -> Arc<dyn Fn() -> i64 + Send + Sync> {
        Arc::new(move || i64::from(now.load(Ordering::SeqCst)) * 1000)
    }

    fn clock_ms(now: Arc<AtomicI64>) -> Arc<dyn Fn() -> i64 + Send + Sync> {
        Arc::new(move || now.load(Ordering::SeqCst))
    }

    struct Leaky(&'static str);

    impl Serialize for Leaky {
        fn serialize<S: Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
            Err(S::Error::custom(self.0))
        }
    }

    fn open_cache(entries: usize, bytes: usize) -> TypedCache<MemoryCache> {
        TypedCache::new(
            MemoryCache::new(MemoryLimits {
                max_entries: entries,
                max_bytes: bytes,
            })
            .expect("limits"),
            1,
        )
    }

    #[tokio::test]
    async fn ttl_namespace_and_version_stay_isolated() {
        let now = Arc::new(AtomicU32::new(10));
        let memory = MemoryCache::with_now(
            MemoryLimits {
                max_entries: 8,
                max_bytes: 4096,
            },
            clock(now.clone()),
        )
        .expect("memory");
        let cache = TypedCache::new(memory, 1);
        let note = Note {
            label: "alpha".into(),
        };
        cache
            .set("tenant.a", "note", &note, Some(Duration::from_secs(5)))
            .await
            .expect("set");
        cache
            .set(
                "tenant.b",
                "note",
                &Note {
                    label: "beta".into(),
                },
                None,
            )
            .await
            .expect("other");
        let loaded: Note = cache
            .get("tenant.a", "note")
            .await
            .expect("get")
            .expect("hit");
        assert_eq!(loaded, note);
        now.store(16, Ordering::SeqCst);
        assert!(cache
            .get::<Note>("tenant.a", "note")
            .await
            .expect("expired")
            .is_none());
        let other: Note = cache
            .get("tenant.b", "note")
            .await
            .expect("kept")
            .expect("still there");
        assert_eq!(other.label, "beta");
        cache.invalidate_namespace("tenant.b").await.expect("drop");
        assert!(cache
            .get::<Note>("tenant.b", "note")
            .await
            .expect("gone")
            .is_none());

        let raw = b"FXC1\x00\x00\x00\x02{}".to_vec();
        cache
            .backend()
            .put_raw("tenant.a", "old", raw, None)
            .await
            .expect("raw");
        assert_eq!(
            cache.get::<Note>("tenant.a", "old").await,
            Err(CacheError::VersionMismatch)
        );
        cache
            .backend()
            .put_raw("tenant.a", "bad", b"nope".to_vec(), None)
            .await
            .expect("bad");
        assert_eq!(
            cache.get::<Note>("tenant.a", "bad").await,
            Err(CacheError::Corrupt)
        );
        assert_eq!(
            cache
                .set("bad key", "note", &note, None)
                .await
                .expect_err("namespace"),
            CacheError::InvalidKey
        );
    }

    #[tokio::test]
    async fn memory_bounds_and_separate_instances() {
        let cache = open_cache(2, 256);
        cache.set("app", "one", &"alpha", None).await.expect("one");
        cache.set("app", "two", &"beta", None).await.expect("two");
        cache
            .set("app", "three", &"gamma", None)
            .await
            .expect("three");
        let left: Option<String> = cache.get("app", "one").await.expect("read");
        let kept: String = cache
            .get("app", "three")
            .await
            .expect("read")
            .expect("kept");
        assert!(left.is_none());
        assert_eq!(kept, "gamma");
        let huge = "x".repeat(300);
        assert_eq!(
            cache.set("app", "huge", &huge, None).await,
            Err(CacheError::TooLarge)
        );
        let other = open_cache(4, 256);
        assert!(other
            .get::<String>("app", "three")
            .await
            .expect("other process")
            .is_none());
    }

    #[tokio::test]
    async fn concurrent_get_or_set_runs_the_producer_once() {
        let cache = Arc::new(open_cache(16, 4096));
        let calls = Arc::new(AtomicU32::new(0));
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let cache = cache.clone();
            let calls = calls.clone();
            tasks.push(tokio::spawn(async move {
                cache
                    .get_or_set("app", "shared", Some(Duration::from_secs(30)), || {
                        let calls = calls.clone();
                        async move {
                            calls.fetch_add(1, Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_millis(40)).await;
                            Ok(Note {
                                label: "once".into(),
                            })
                        }
                    })
                    .await
            }));
        }
        for task in tasks {
            let note = task.await.expect("join").expect("value");
            assert_eq!(note.label, "once");
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn fragment_cache_invalidates_only_its_namespace() {
        let memory = MemoryCache::new(MemoryLimits {
            max_entries: 16,
            max_bytes: 4096,
        })
        .expect("memory");
        let records = TypedCache::new(memory.clone(), 1);
        let fragments = FragmentCache::new(TypedCache::new(memory, 1), "home").expect("view");
        records
            .set("records", "home", &"data", None)
            .await
            .expect("data");
        let calls = AtomicU32::new(0);
        let first = fragments
            .render("index", Duration::from_secs(60), || {
                calls.fetch_add(1, Ordering::SeqCst);
                async { Ok("page".to_owned()) }
            })
            .await
            .expect("render");
        let second = fragments
            .render("index", Duration::from_secs(60), || {
                calls.fetch_add(1, Ordering::SeqCst);
                async { Ok("other".to_owned()) }
            })
            .await
            .expect("cached");
        assert_eq!(first, second);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        fragments.invalidate_all().await.expect("view namespace");
        let kept: String = records
            .get("records", "home")
            .await
            .expect("untouched")
            .expect("data remains");
        assert_eq!(kept, "data");
        let third = fragments
            .render("index", Duration::from_secs(60), || async {
                Ok("fresh".to_owned())
            })
            .await
            .expect("fresh");
        assert_eq!(third, "fresh");
    }

    #[tokio::test]
    async fn ttl_boundary_frees_bytes_and_eviction_is_deterministic() {
        let now = Arc::new(AtomicU32::new(10));
        let memory = MemoryCache::with_now(
            MemoryLimits {
                max_entries: 2,
                max_bytes: 48,
            },
            clock(now.clone()),
        )
        .expect("memory");
        let cache = TypedCache::new(memory, 1);
        let value = "x".repeat(20);
        cache
            .set("app", "live", &value, Some(Duration::from_secs(5)))
            .await
            .expect("set");
        now.store(15, Ordering::SeqCst);
        assert!(cache
            .get::<String>("app", "live")
            .await
            .expect("boundary")
            .is_none());
        cache
            .set("app", "after", &value, None)
            .await
            .expect("bytes released by expiry");

        let frozen = Arc::new(AtomicU32::new(5));
        let memory = MemoryCache::with_now(
            MemoryLimits {
                max_entries: 2,
                max_bytes: 4096,
            },
            clock(frozen),
        )
        .expect("frozen");
        let cache = TypedCache::new(memory, 1);
        cache.set("app", "b", &"beta", None).await.expect("b");
        cache.set("app", "c", &"gamma", None).await.expect("c");
        cache.set("app", "a", &"alpha", None).await.expect("a");
        assert!(cache
            .get::<String>("app", "b")
            .await
            .expect("evicted")
            .is_none());
        assert_eq!(
            cache
                .get::<String>("app", "c")
                .await
                .expect("kept")
                .unwrap(),
            "gamma"
        );
        cache.invalidate_namespace("app").await.expect("drop");
        cache
            .set("app", "again", &value, None)
            .await
            .expect("bytes released by invalidation");
        assert!(cache
            .get::<String>("app", "c")
            .await
            .expect("namespace")
            .is_none());
        let other = TypedCache::new(
            MemoryCache::new(MemoryLimits {
                max_entries: 2,
                max_bytes: 4096,
            })
            .expect("other"),
            1,
        );
        assert!(other
            .get::<String>("app", "again")
            .await
            .expect("isolated")
            .is_none());
    }

    #[tokio::test]
    async fn instant_get_or_set_does_not_drop_waiters() {
        let cache = Arc::new(open_cache(32, 8192));
        for round in 0..40 {
            let calls = Arc::new(AtomicU32::new(0));
            let mut tasks = Vec::new();
            for _ in 0..8 {
                let cache = cache.clone();
                let calls = calls.clone();
                let key = format!("fast-{round}");
                tasks.push(tokio::spawn(async move {
                    cache
                        .get_or_set("app", &key, None, || {
                            let calls = calls.clone();
                            async move {
                                calls.fetch_add(1, Ordering::SeqCst);
                                Ok::<_, CacheError>("ready".to_owned())
                            }
                        })
                        .await
                }));
            }
            let finished = tokio::time::timeout(Duration::from_secs(2), async {
                let mut values = Vec::new();
                for task in tasks {
                    values.push(task.await.expect("join").expect("value"));
                }
                values
            })
            .await
            .expect("waiters finished");
            assert!(finished.iter().all(|value| value == "ready"));
            assert_eq!(calls.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn parallel_reads_and_writes_release_bytes() {
        let cache = Arc::new(open_cache(64, 4096));
        let mut tasks = Vec::new();
        for index in 0..16 {
            let cache = cache.clone();
            tasks.push(tokio::spawn(async move {
                let key = format!("k{index}");
                cache
                    .set("hammer", &key, &format!("value-{index}"), None)
                    .await
                    .expect("set");
                let loaded: Option<String> = cache.get("hammer", &key).await.expect("get");
                assert!(loaded.is_some());
                cache.delete("hammer", &key).await.expect("delete");
            }));
        }
        for task in tasks {
            task.await.expect("join");
        }
        cache
            .invalidate_namespace("hammer")
            .await
            .expect("invalidate");
        for index in 0..16 {
            assert!(cache
                .get::<String>("hammer", &format!("k{index}"))
                .await
                .expect("cleared")
                .is_none());
        }
        let big = "x".repeat(2500);
        cache
            .set("hammer", "full", &big, None)
            .await
            .expect("fits after release");
        cache.delete("hammer", "full").await.expect("free");
        cache
            .set("hammer", "full", &big, None)
            .await
            .expect("reused");
        let other = open_cache(8, 4096);
        assert!(other
            .get::<String>("hammer", "full")
            .await
            .expect("isolated")
            .is_none());
    }

    #[tokio::test]
    async fn lookup_distinguishes_miss_expiry_and_a_second_read() {
        let now = Arc::new(AtomicI64::new(1_000));
        let cache = TypedCache::new(
            MemoryCache::with_now(
                MemoryLimits {
                    max_entries: 4,
                    max_bytes: 1024,
                },
                clock_ms(now.clone()),
            )
            .expect("memory"),
            1,
        );
        assert_eq!(
            cache
                .lookup::<String>("app", "missing")
                .await
                .expect("miss"),
            CacheLookup::Miss
        );
        cache
            .delete("app", "missing")
            .await
            .expect("delete of a miss is success");
        cache
            .set("app", "live", &"value", Some(Duration::from_nanos(1)))
            .await
            .expect("short ttl");
        assert_eq!(
            cache.lookup::<String>("app", "live").await.expect("hit"),
            CacheLookup::Hit("value".to_owned())
        );
        now.store(1_001, Ordering::SeqCst);
        assert_eq!(
            cache
                .lookup::<String>("app", "live")
                .await
                .expect("expired"),
            CacheLookup::Expired
        );
        assert!(cache
            .get::<String>("app", "live")
            .await
            .expect("gone")
            .is_none());
        assert_eq!(
            cache
                .lookup::<String>("app", "live")
                .await
                .expect("second read"),
            CacheLookup::Miss
        );

        cache
            .backend()
            .put_raw(
                "app",
                "garbage",
                b"nope".to_vec(),
                Some(Duration::from_millis(5)),
            )
            .await
            .expect("raw");
        now.store(1_010, Ordering::SeqCst);
        assert_eq!(
            cache
                .lookup::<String>("app", "garbage")
                .await
                .expect("expired before decode"),
            CacheLookup::Expired
        );
    }

    #[tokio::test]
    async fn zero_ttl_and_zero_limits_are_rejected() {
        assert!(matches!(
            MemoryCache::new(MemoryLimits {
                max_entries: 0,
                max_bytes: 32,
            }),
            Err(CacheError::InvalidLimits)
        ));
        assert!(matches!(
            MemoryCache::new(MemoryLimits {
                max_entries: 1,
                max_bytes: 0,
            }),
            Err(CacheError::InvalidLimits)
        ));
        let cache = open_cache(4, 256);
        assert_eq!(
            cache.set("app", "zero", &"x", Some(Duration::ZERO)).await,
            Err(CacheError::InvalidTtl)
        );
        assert_eq!(
            cache
                .lookup::<String>("app", "zero")
                .await
                .expect("not stored"),
            CacheLookup::Miss
        );
        let calls = AtomicU32::new(0);
        assert_eq!(
            cache
                .get_or_set("app", "zero", Some(Duration::ZERO), || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    async { Ok::<_, CacheError>("nope".to_owned()) }
                })
                .await,
            Err(CacheError::InvalidTtl)
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn namespaces_and_keys_do_not_collide() {
        let cache = open_cache(8, 1024);
        cache.set("ab", "c", &"left", None).await.expect("left");
        cache.set("a", "bc", &"right", None).await.expect("right");
        cache
            .set("tenant", "profile", &"shared", None)
            .await
            .expect("short");
        cache
            .set("tenant.a", "profile", &"isolated", None)
            .await
            .expect("long");
        assert_eq!(
            cache.get::<String>("ab", "c").await.expect("read").unwrap(),
            "left"
        );
        assert_eq!(
            cache.get::<String>("a", "bc").await.expect("read").unwrap(),
            "right"
        );
        cache.invalidate_namespace("tenant").await.expect("exact");
        assert!(cache
            .get::<String>("tenant", "profile")
            .await
            .expect("cleared")
            .is_none());
        assert_eq!(
            cache
                .get::<String>("tenant.a", "profile")
                .await
                .expect("kept")
                .unwrap(),
            "isolated"
        );
        let dotted = crate::view::user_storage_key("a.b", "c").expect("dotted");
        let split = crate::view::user_storage_key("a", "b.c").expect("split");
        let colon = crate::view::user_storage_key("a:b", "c").expect("colon");
        let tail = crate::view::user_storage_key("a", "b:c").expect("tail");
        assert_ne!(dotted, split);
        assert_ne!(colon, tail);
        let long_user = "u".repeat(170);
        let long_part = "k".repeat(20);
        assert!(crate::view::user_storage_key(&long_user, &long_part).is_err());
    }

    #[tokio::test]
    async fn shared_namespace_does_not_isolate_tenants() {
        let cache = open_cache(4, 1024);
        cache
            .set("shop", "profile", &"tenant-a", None)
            .await
            .expect("first");
        cache
            .set("shop", "profile", &"tenant-b", None)
            .await
            .expect("overwrite");
        assert_eq!(
            cache
                .get::<String>("shop", "profile")
                .await
                .expect("mixed")
                .unwrap(),
            "tenant-b"
        );
    }

    #[tokio::test]
    async fn confidential_payload_stays_in_its_namespace_and_out_of_errors() {
        let secret = "session-token-do-not-log";
        let cache = open_cache(4, 1024);
        let error = cache
            .set("user.ada", "secret", &Leaky(secret), None)
            .await
            .expect_err("serialize");
        assert_eq!(error, CacheError::Serialization);
        assert!(!error.to_string().contains(secret));
        assert!(!format!("{error:?}").contains(secret));
        assert_eq!(
            cache
                .lookup::<String>("user.ada", "secret")
                .await
                .expect("not stored"),
            CacheLookup::Miss
        );
        cache
            .set("user.ada", "profile", &secret, None)
            .await
            .expect("caller stored it");
        assert_eq!(
            cache
                .get::<String>("user.ada", "profile")
                .await
                .expect("ada")
                .as_deref(),
            Some(secret)
        );
        assert!(cache
            .get::<String>("user.bob", "profile")
            .await
            .expect("bob")
            .is_none());
        let mismatch = cache
            .get::<Note>("user.ada", "profile")
            .await
            .expect_err("type");
        assert_eq!(mismatch, CacheError::Corrupt);
        assert!(!mismatch.to_string().contains(secret));
        assert!(!format!("{mismatch:?}").contains(secret));
        assert!(!CacheError::AuthenticatedContent
            .to_string()
            .contains(secret));
    }

    #[tokio::test]
    async fn version_mismatch_does_not_recompute() {
        let cache = open_cache(4, 1024);
        cache
            .backend()
            .put_raw("app", "old", b"FXC1\x00\x00\x00\x02{}".to_vec(), None)
            .await
            .expect("raw");
        let calls = AtomicU32::new(0);
        assert_eq!(
            cache
                .get_or_set("app", "old", None, || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    async {
                        Ok(Note {
                            label: "fresh".into(),
                        })
                    }
                })
                .await,
            Err(CacheError::VersionMismatch)
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(!CacheError::VersionMismatch.to_string().contains("fresh"));
    }

    #[tokio::test]
    async fn cloned_memory_shares_bytes_but_not_stampede_locks() {
        let memory = MemoryCache::new(MemoryLimits {
            max_entries: 8,
            max_bytes: 4096,
        })
        .expect("memory");
        let first = Arc::new(TypedCache::new(memory.clone(), 1));
        let second = Arc::new(TypedCache::new(memory, 1));
        let calls = Arc::new(AtomicU32::new(0));
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let left = {
            let cache = first.clone();
            let calls = calls.clone();
            let barrier = barrier.clone();
            tokio::spawn(async move {
                cache
                    .get_or_set("app", "shared", None, || {
                        let calls = calls.clone();
                        let barrier = barrier.clone();
                        async move {
                            barrier.wait().await;
                            calls.fetch_add(1, Ordering::SeqCst);
                            Ok::<_, CacheError>("same".to_owned())
                        }
                    })
                    .await
            })
        };
        let right = {
            let cache = second.clone();
            let calls = calls.clone();
            let barrier = barrier.clone();
            tokio::spawn(async move {
                cache
                    .get_or_set("app", "shared", None, || {
                        let calls = calls.clone();
                        let barrier = barrier.clone();
                        async move {
                            barrier.wait().await;
                            calls.fetch_add(1, Ordering::SeqCst);
                            Ok::<_, CacheError>("same".to_owned())
                        }
                    })
                    .await
            })
        };
        assert_eq!(left.await.expect("join").expect("left"), "same");
        assert_eq!(right.await.expect("join").expect("right"), "same");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            first
                .get::<String>("app", "shared")
                .await
                .expect("visible")
                .as_deref(),
            Some("same")
        );
        let isolated = open_cache(4, 1024);
        assert!(isolated
            .get::<String>("app", "shared")
            .await
            .expect("other instance")
            .is_none());
    }

    #[tokio::test]
    async fn separate_instances_each_run_their_own_producer() {
        let left = Arc::new(open_cache(8, 1024));
        let right = Arc::new(open_cache(8, 1024));
        let calls = Arc::new(AtomicU32::new(0));
        let mut tasks = Vec::new();
        for cache in [left.clone(), right.clone()] {
            let calls = calls.clone();
            tasks.push(tokio::spawn(async move {
                cache
                    .get_or_set("app", "page", None, || {
                        let calls = calls.clone();
                        async move {
                            calls.fetch_add(1, Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_millis(30)).await;
                            Ok::<_, CacheError>("local".to_owned())
                        }
                    })
                    .await
            }));
        }
        for task in tasks {
            assert_eq!(task.await.expect("join").expect("value"), "local");
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(right
            .get::<String>("app", "page")
            .await
            .expect("own")
            .is_some());
        assert!(open_cache(4, 1024)
            .get::<String>("app", "page")
            .await
            .expect("third")
            .is_none());
    }

    #[tokio::test]
    async fn producer_failure_is_coalesced_and_then_retried() {
        let cache = Arc::new(open_cache(8, 1024));
        let calls = Arc::new(AtomicU32::new(0));
        let mut tasks = Vec::new();
        for _ in 0..6 {
            let cache = cache.clone();
            let calls = calls.clone();
            tasks.push(tokio::spawn(async move {
                cache
                    .get_or_set("app", "boom", None, || {
                        let calls = calls.clone();
                        async move {
                            calls.fetch_add(1, Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_millis(30)).await;
                            Err::<String, _>(CacheError::Backend)
                        }
                    })
                    .await
            }));
        }
        for task in tasks {
            assert_eq!(task.await.expect("join"), Err(CacheError::Backend));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(!CacheError::Backend.to_string().contains("boom"));
        let value = cache
            .get_or_set("app", "boom", None, || {
                let calls = calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, CacheError>("recovered".to_owned())
                }
            })
            .await
            .expect("retry");
        assert_eq!(value, "recovered");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn view_cache_refuses_shared_authenticated_output() {
        let memory = MemoryCache::new(MemoryLimits {
            max_entries: 8,
            max_bytes: 4096,
        })
        .expect("memory");
        let cache = ViewCache::new(TypedCache::new(memory, 1), "home").expect("view");
        let calls = AtomicU32::new(0);
        let first = cache
            .render(
                "index",
                Duration::from_secs(30),
                &ViewScope::Anonymous,
                false,
                || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    async { Ok("public".to_owned()) }
                },
            )
            .await
            .expect("public");
        assert_eq!(first, "public");
        let refused = cache
            .render(
                "index",
                Duration::from_secs(30),
                &ViewScope::Anonymous,
                true,
                || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    async { Ok("private-session".to_owned()) }
                },
            )
            .await
            .expect_err("authenticated");
        assert_eq!(refused, CacheError::AuthenticatedContent);
        assert!(!refused.to_string().contains("private-session"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let again = cache
            .render(
                "index",
                Duration::from_secs(30),
                &ViewScope::Anonymous,
                false,
                || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    async { Ok("replaced".to_owned()) }
                },
            )
            .await
            .expect("still public");
        assert_eq!(again, "public");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(ViewCache::<MemoryCache>::new(
            TypedCache::new(
                MemoryCache::new(MemoryLimits {
                    max_entries: 1,
                    max_bytes: 64,
                })
                .expect("memory"),
                1,
            ),
            "bad name",
        )
        .is_err());
    }

    #[tokio::test]
    async fn view_cache_isolates_users_and_invalidation() {
        let memory = MemoryCache::new(MemoryLimits {
            max_entries: 16,
            max_bytes: 8192,
        })
        .expect("memory");
        let fragments =
            FragmentCache::new(TypedCache::new(memory.clone(), 1), "notes").expect("fragment");
        let cache = ViewCache::new(TypedCache::new(memory, 1), "notes").expect("view");
        let fragment_calls = AtomicU32::new(0);
        fragments
            .render("list", Duration::from_secs(30), || {
                fragment_calls.fetch_add(1, Ordering::SeqCst);
                async { Ok("fragment".to_owned()) }
            })
            .await
            .expect("fragment");
        let calls = AtomicU32::new(0);
        let ada = cache
            .render(
                "list",
                Duration::from_secs(30),
                &ViewScope::User {
                    user_id: "a.b".into(),
                },
                true,
                || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    async { Ok("ada".to_owned()) }
                },
            )
            .await
            .expect("ada");
        let bob = cache
            .render(
                "list",
                Duration::from_secs(30),
                &ViewScope::User {
                    user_id: "a".into(),
                },
                true,
                || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    async { Ok("bob".to_owned()) }
                },
            )
            .await
            .expect("bob");
        assert_eq!(ada, "ada");
        assert_eq!(bob, "bob");
        let ada_again = cache
            .render(
                "list",
                Duration::from_secs(30),
                &ViewScope::User {
                    user_id: "a.b".into(),
                },
                true,
                || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    async { Ok("other".to_owned()) }
                },
            )
            .await
            .expect("cached ada");
        assert_eq!(ada_again, "ada");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        let skipped = AtomicU32::new(0);
        assert_eq!(
            cache
                .render(
                    "list",
                    Duration::from_secs(30),
                    &ViewScope::User {
                        user_id: "bad id".into(),
                    },
                    true,
                    || {
                        skipped.fetch_add(1, Ordering::SeqCst);
                        async { Ok("nope".to_owned()) }
                    },
                )
                .await,
            Err(CacheError::InvalidKey)
        );
        assert_eq!(skipped.load(Ordering::SeqCst), 0);
        cache.invalidate_anonymous().await.expect("public");
        fragments
            .render("list", Duration::from_secs(30), || {
                fragment_calls.fetch_add(1, Ordering::SeqCst);
                async { Ok("fresh-fragment".to_owned()) }
            })
            .await
            .expect("rendered again");
        assert_eq!(fragment_calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            cache
                .render(
                    "list",
                    Duration::from_secs(30),
                    &ViewScope::User {
                        user_id: "a.b".into(),
                    },
                    true,
                    || async { Ok("wiped".to_owned()) },
                )
                .await
                .expect("user remains"),
            "ada"
        );
        cache
            .invalidate(
                "list",
                &ViewScope::User {
                    user_id: "a.b".into(),
                },
            )
            .await
            .expect("one user");
        assert_eq!(
            cache
                .render(
                    "list",
                    Duration::from_secs(30),
                    &ViewScope::User {
                        user_id: "a".into(),
                    },
                    true,
                    || async { Ok("bob-replaced".to_owned()) },
                )
                .await
                .expect("bob remains"),
            "bob"
        );
        cache.invalidate_users().await.expect("users");
        assert_eq!(
            cache
                .render(
                    "list",
                    Duration::from_secs(30),
                    &ViewScope::User {
                        user_id: "a".into(),
                    },
                    true,
                    || async { Ok("bob-new".to_owned()) },
                )
                .await
                .expect("rebuilt"),
            "bob-new"
        );
    }

    #[tokio::test]
    async fn unsafe_keys_are_rejected() {
        let cache = open_cache(4, 256);
        let samples = [
            ("", "ok"),
            ("_hidden", "ok"),
            ("bad key", "ok"),
            ("tenant", ""),
            ("tenant", "a b"),
            ("tenant", ".."),
            ("tenant", "a\\b"),
            ("tenant", "line\n"),
        ];
        for (namespace, key) in samples {
            assert_eq!(
                cache.set(namespace, key, &"x", None).await,
                Err(CacheError::InvalidKey),
                "{namespace:?} {key:?}"
            );
        }
        let long_namespace = "n".repeat(65);
        let long_key = "k".repeat(181);
        assert_eq!(
            cache.set(&long_namespace, "ok", &"x", None).await,
            Err(CacheError::InvalidKey)
        );
        assert_eq!(
            cache.set("tenant", &long_key, &"x", None).await,
            Err(CacheError::InvalidKey)
        );
    }
}
