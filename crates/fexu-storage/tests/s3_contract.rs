#![cfg(feature = "s3-storage")]

use fexu_storage::{
    credentials_from_env, prepare_s3, S3Credentials, S3Limits, S3Storage, Storage, StorageError,
    ACCESS_KEY_ENV, SECRET_KEY_ENV,
};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

fn fixture_plan() -> fexu_storage::S3Plan {
    let endpoint = std::env::var("FEXU_TEST_S3_ENDPOINT").expect("isolated fixture endpoint");
    assert!(endpoint.starts_with("https://127.0.0.1:"));
    prepare_s3(
        &endpoint,
        "us-east-1",
        "fexu-test-media",
        ACCESS_KEY_ENV,
        SECRET_KEY_ENV,
    )
    .unwrap()
}

fn fixture() -> S3Storage {
    let plan = fixture_plan();
    let ca = std::fs::read(std::env::var_os("FEXU_TEST_S3_CA").unwrap()).unwrap();
    S3Storage::new(
        &plan,
        credentials_from_env(&plan).unwrap(),
        S3Limits {
            max_bytes: 128,
            ..S3Limits::default()
        },
        &["text/plain"],
        Some(&ca),
    )
    .unwrap()
}

#[tokio::test]
#[ignore = "requires isolated TLS S3 fixture"]
async fn s3_storage_contract() {
    let first = fixture();
    let second = fixture();
    let owner = Uuid::new_v4();
    let other = Uuid::new_v4();
    let (left, right) = tokio::join!(
        first.save(owner, "../note.txt", b"left"),
        second.save(owner, "note.txt", b"right")
    );
    let left = left.unwrap();
    let right = right.unwrap();
    assert_ne!(left.id, right.id);
    assert_eq!(left.original_name, "note.txt");
    assert_eq!(
        second.read(owner, left.id).await.unwrap(),
        (left.clone(), b"left".to_vec())
    );
    assert_eq!(first.read(owner, right.id).await.unwrap().1, b"right");
    assert_eq!(
        second.read(other, left.id).await,
        Err(StorageError::NotFound)
    );
    assert_eq!(
        first.read_for(other, owner, left.id).await,
        Err(StorageError::Forbidden)
    );
    assert_eq!(
        first.save(owner, "note.txt", &[b'x'; 129]).await,
        Err(StorageError::TooLarge)
    );
    assert_eq!(
        first.save(owner, "note.pdf", b"text").await,
        Err(StorageError::TypeRejected)
    );
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let staged = first
        .save_expiring(owner, "stage.txt", b"stage", Some(now + 300))
        .await
        .unwrap();
    let retained = second.retain(owner, staged.id).await.unwrap();
    assert_eq!(retained.expires_at, None);
    assert_eq!(first.metadata(owner, staged.id).await.unwrap(), retained);
    let expired = first
        .save_expiring(owner, "expired.txt", b"expired", Some(now - 1))
        .await
        .unwrap();
    assert_eq!(
        second.read(owner, expired.id).await,
        Err(StorageError::Expired)
    );
    assert!(!second.exists(owner, expired.id).await.unwrap());
    assert_eq!(
        second.retain(owner, expired.id).await,
        Err(StorageError::Expired)
    );
    for file in [left, right, staged, expired] {
        second.delete(owner, file.id).await.unwrap();
        assert!(!first.exists(owner, file.id).await.unwrap());
    }
}

#[tokio::test]
#[ignore = "requires isolated TLS S3 fixture"]
async fn s3_rejects_untrusted_tls_and_wrong_credentials() {
    let plan = fixture_plan();
    let ca = std::fs::read(std::env::var_os("FEXU_TEST_S3_CA").unwrap()).unwrap();
    let untrusted = S3Storage::new(
        &plan,
        credentials_from_env(&plan).unwrap(),
        S3Limits::default(),
        &["text/plain"],
        None,
    )
    .unwrap();
    let denied = S3Storage::new(
        &plan,
        S3Credentials {
            access_key: "invalid-access-key".to_owned(),
            secret_key: "invalid-secret-key".to_owned(),
        },
        S3Limits::default(),
        &["text/plain"],
        Some(&ca),
    )
    .unwrap();
    let owner = Uuid::new_v4();
    assert_eq!(
        untrusted.save(owner, "test.txt", b"test").await,
        Err(StorageError::Failed)
    );
    assert_eq!(
        denied.save(owner, "test.txt", b"test").await,
        Err(StorageError::Failed)
    );
}

#[tokio::test]
#[ignore = "requires isolated TLS S3 fixture"]
async fn s3_retain_delete_race() {
    let first = fixture();
    let second = fixture();
    let owner = Uuid::new_v4();
    let deadline = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
        + 300;
    for _ in 0..8 {
        let staged = first
            .save_expiring(owner, "race.txt", b"race", Some(deadline))
            .await
            .unwrap();
        let (retained, deleted) = tokio::join!(
            first.retain(owner, staged.id),
            second.delete(owner, staged.id)
        );
        deleted.unwrap();
        match retained {
            Ok(meta) => assert_eq!(meta.expires_at, None),
            Err(StorageError::NotFound | StorageError::Failed) => {}
            Err(error) => panic!("unexpected retain error: {error:?}"),
        }
        assert!(!first.exists(owner, staged.id).await.unwrap());
        assert_eq!(
            second.retain(owner, staged.id).await,
            Err(StorageError::NotFound)
        );
    }
}

#[tokio::test]
#[ignore = "requires isolated TLS S3 fixture and restart phase"]
async fn s3_restart_contract() {
    let storage = fixture();
    let state =
        std::env::var_os("FEXU_TEST_S3_RESTART_STATE").expect("isolated restart state path");
    let phase = std::env::var("FEXU_TEST_S3_RESTART_PHASE").unwrap();
    match phase.as_str() {
        "seed" => {
            let owner = Uuid::new_v4();
            let file = storage
                .save(owner, "restart.txt", b"persistent bytes")
                .await
                .unwrap();
            std::fs::write(state, format!("{} {}", owner, file.id)).unwrap();
        }
        "verify" => {
            let saved = std::fs::read_to_string(state).unwrap();
            let (owner, id) = saved.split_once(' ').unwrap();
            let owner = Uuid::parse_str(owner).unwrap();
            let id = Uuid::parse_str(id).unwrap();
            let (meta, bytes) = storage.read(owner, id).await.unwrap();
            assert_eq!(meta.owner, owner);
            assert_eq!(meta.id, id);
            assert_eq!(meta.original_name, "restart.txt");
            assert_eq!(bytes, b"persistent bytes");
            storage.delete(owner, id).await.unwrap();
            assert!(!storage.exists(owner, id).await.unwrap());
        }
        _ => panic!("invalid restart phase"),
    }
}

#[tokio::test]
#[ignore = "requires isolated TLS S3 fixture"]
async fn s3_expiry_cleanup_contract() {
    let first = fixture();
    let second = fixture();
    let owner = Uuid::new_v4();
    let other = Uuid::new_v4();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let expired = first
        .save_expiring(owner, "expired.txt", b"expired", Some(now - 1))
        .await
        .unwrap();
    let staged = first
        .save_expiring(owner, "staged.txt", b"staged", Some(now + 300))
        .await
        .unwrap();
    let permanent = second.retain(owner, staged.id).await.unwrap();
    let future = first
        .save_expiring(owner, "future.txt", b"future", Some(now + 300))
        .await
        .unwrap();

    assert!(!second
        .cleanup_unreferenced_expired(other, expired.id)
        .await
        .unwrap());
    assert!(!second
        .cleanup_unreferenced_expired(owner, permanent.id)
        .await
        .unwrap());
    assert!(!second
        .cleanup_unreferenced_expired(owner, future.id)
        .await
        .unwrap());
    assert_eq!(first.read(owner, permanent.id).await.unwrap().1, b"staged");
    assert_eq!(first.read(owner, future.id).await.unwrap().1, b"future");
    assert!(second
        .cleanup_unreferenced_expired(owner, expired.id)
        .await
        .unwrap());
    assert_eq!(
        first.metadata(owner, expired.id).await,
        Err(StorageError::NotFound)
    );
    assert_eq!(
        first.retain(owner, expired.id).await,
        Err(StorageError::NotFound)
    );
    assert!(!first
        .cleanup_unreferenced_expired(owner, expired.id)
        .await
        .unwrap());
    for file in [permanent, future] {
        first.delete(owner, file.id).await.unwrap();
    }
}
