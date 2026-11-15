use fexu::cache::{CacheError, CacheLookup};
use notes::cache_page::{public_list_markup, NoteCache};

#[tokio::test]
async fn note_cache_isolates_tenants_users_and_instances() {
    let cache = NoteCache::new(16, 8192).expect("cache");
    assert_eq!(
        cache.summary_state("tenant.ada").await.expect("state"),
        CacheLookup::Miss
    );
    cache
        .remember_summary("tenant.ada", "ada-notes")
        .await
        .expect("remember");
    assert_eq!(
        cache.summary("tenant.ada").await.expect("ada").as_deref(),
        Some("ada-notes")
    );
    assert_eq!(
        cache.summary_state("tenant.ada").await.expect("hit"),
        CacheLookup::Hit("ada-notes".to_owned())
    );
    assert!(cache.summary("tenant.bob").await.expect("bob").is_none());
    cache.invalidate_tenant("tenant.ada").await.expect("drop");
    assert!(cache
        .summary("tenant.ada")
        .await
        .expect("cleared")
        .is_none());

    let html = cache.public_list(false).await.expect("public");
    assert_eq!(html, public_list_markup());
    assert_eq!(cache.public_list(false).await.expect("hit"), html);
    assert_eq!(cache.page_renders(), 1);
    assert_eq!(
        cache.public_list(true).await.expect_err("signed in"),
        CacheError::AuthenticatedContent
    );
    assert_eq!(cache.page_renders(), 1);
    cache.invalidate_public_list().await.expect("invalidate");
    assert_eq!(cache.public_list(false).await.expect("fresh"), html);
    assert_eq!(cache.page_renders(), 2);

    assert_eq!(
        cache
            .user_list("ada", || async { Ok("ada-page".to_owned()) })
            .await
            .expect("ada page"),
        "ada-page"
    );
    assert_eq!(
        cache
            .user_list("ada", || async { Ok("replaced".to_owned()) })
            .await
            .expect("cached ada"),
        "ada-page"
    );
    assert_eq!(
        cache
            .user_list("bob", || async { Ok("bob-page".to_owned()) })
            .await
            .expect("bob page"),
        "bob-page"
    );
    assert_eq!(cache.page_renders(), 4);
    assert_eq!(
        cache.public_list(false).await.expect("public remains"),
        html
    );
    assert_eq!(cache.page_renders(), 4);

    let other = NoteCache::new(16, 8192).expect("second process");
    assert!(other
        .summary("tenant.ada")
        .await
        .expect("isolated")
        .is_none());
    assert_eq!(
        other.public_list(false).await.expect("own page"),
        public_list_markup()
    );
    assert_eq!(other.page_renders(), 1);
    assert_eq!(cache.page_renders(), 4);
}
