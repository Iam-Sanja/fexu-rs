//! File and image field lifecycle.
//!
//! `prepare_media` validates and writes nothing. `stage` stores new bytes under
//! a fresh id and leaves the previous file in place. A failed form or database
//! transaction calls `abort`, which deletes only those new ids. After the
//! database commit, `commit` clears the staging expiry and then deletes the
//! previous files. A database rollback does not reach the storage backend.
//! A failed delete stays in [`CommitReport::removal_failed`] and is still readable.

use crate::{
    detect_type, validate_image, ImageLimits, ImageValidation, Storage, StorageAccess,
    StorageAction, StorageError, StoredFile,
};
use uuid::Uuid;

/// Longest staging lifetime, in seconds. Longer values are rejected.
pub const MAX_STAGING_TTL: i64 = 24 * 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    File,
    Image,
}

/// Owner and file id. This is not a filesystem path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaRef {
    pub owner: Uuid,
    pub id: Uuid,
}

#[derive(Debug, Clone, Copy)]
pub struct MediaPolicy {
    pub max_bytes: u64,
    pub max_request_bytes: u64,
    pub file_types: &'static [&'static str],
    pub image_limits: ImageLimits,
}

pub struct IncomingFile<'a> {
    pub original_name: &'a str,
    pub declared_type: Option<&'a str>,
    pub bytes: &'a [u8],
}

pub struct FieldSpec<'a> {
    pub name: &'a str,
    pub kind: MediaKind,
    pub required: bool,
    pub readonly: bool,
    pub existing: Option<MediaRef>,
    pub clear: bool,
    pub incoming: Option<IncomingFile<'a>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldError {
    pub field: String,
    pub code: &'static str,
}

#[derive(Debug)]
pub struct UploadPlan {
    owner: Uuid,
    decisions: Vec<Decision>,
}

#[derive(Debug)]
enum Decision {
    Absent {
        field: String,
    },
    Keep {
        field: String,
        media: MediaRef,
    },
    Remove {
        field: String,
        media: MediaRef,
    },
    Stage {
        field: String,
        upload: PreparedFile,
        retire: Option<MediaRef>,
    },
}

#[derive(Debug)]
struct PreparedFile {
    original_name: String,
    content_type: String,
    bytes: Vec<u8>,
    pixels_decoded: bool,
}

#[derive(Debug)]
pub enum PlannedField<'a> {
    Absent {
        field: &'a str,
    },
    Keep {
        field: &'a str,
        media: MediaRef,
    },
    Remove {
        field: &'a str,
        media: MediaRef,
    },
    Stage {
        field: &'a str,
        content_type: &'a str,
        size: u64,
        retire: Option<MediaRef>,
        pixels_decoded: bool,
    },
}

#[derive(Debug)]
pub struct StagedMedia {
    owner: Uuid,
    created: Vec<StagedFile>,
    removals: Vec<MediaRef>,
    kept: Vec<BoundMedia>,
}

#[derive(Debug)]
pub struct StagedFile {
    pub field: String,
    pub file: StoredFile,
    pub retire: Option<MediaRef>,
    pub pixels_decoded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundMedia {
    pub field: String,
    pub media: Option<MediaRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbortReport {
    pub removed: Vec<MediaRef>,
    pub remove_failed: Vec<MediaRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitReport {
    pub bound: Vec<BoundMedia>,
    pub removed: Vec<MediaRef>,
    pub removal_failed: Vec<MediaRef>,
    pub retain_failed: Vec<MediaRef>,
}

impl UploadPlan {
    pub fn fields(&self) -> impl Iterator<Item = PlannedField<'_>> {
        self.decisions.iter().map(|decision| match decision {
            Decision::Absent { field } => PlannedField::Absent { field },
            Decision::Keep { field, media } => PlannedField::Keep {
                field,
                media: *media,
            },
            Decision::Remove { field, media } => PlannedField::Remove {
                field,
                media: *media,
            },
            Decision::Stage {
                field,
                upload,
                retire,
            } => PlannedField::Stage {
                field,
                content_type: &upload.content_type,
                size: upload.bytes.len() as u64,
                retire: *retire,
                pixels_decoded: upload.pixels_decoded,
            },
        })
    }

    /// Writes each replacement. A later write failure deletes the files created
    /// by this call. Files that could not be deleted keep their expiry.
    pub async fn stage<S: Storage + ?Sized>(
        self,
        storage: &S,
        now: i64,
        ttl: i64,
    ) -> Result<StagedMedia, StorageError> {
        if !(1..=MAX_STAGING_TTL).contains(&ttl) || now.checked_add(ttl).is_none() {
            return Err(StorageError::Failed);
        }
        let expires_at = now + ttl;
        let mut created = Vec::new();
        let mut removals = Vec::new();
        let mut kept = Vec::new();
        for decision in self.decisions {
            match decision {
                Decision::Absent { field } => kept.push(BoundMedia { field, media: None }),
                Decision::Keep { field, media } => kept.push(BoundMedia {
                    field,
                    media: Some(media),
                }),
                Decision::Remove { field, media } => {
                    kept.push(BoundMedia { field, media: None });
                    removals.push(media);
                }
                Decision::Stage {
                    field,
                    upload,
                    retire,
                } => match storage
                    .save_expiring(
                        self.owner,
                        &upload.original_name,
                        &upload.bytes,
                        Some(expires_at),
                    )
                    .await
                {
                    Ok(file) => {
                        if let Some(previous) = retire {
                            removals.push(previous);
                        }
                        created.push(StagedFile {
                            field,
                            file,
                            retire,
                            pixels_decoded: upload.pixels_decoded,
                        });
                    }
                    Err(error) => {
                        for staged in &created {
                            let _ = storage.delete(self.owner, staged.file.id).await;
                        }
                        return Err(error);
                    }
                },
            }
        }
        Ok(StagedMedia {
            owner: self.owner,
            created,
            removals,
            kept,
        })
    }
}

impl StagedMedia {
    pub fn created(&self) -> &[StagedFile] {
        &self.created
    }

    /// Deletes staged ids. Previous files stay available.
    pub async fn abort<S: Storage + ?Sized>(self, storage: &S) -> AbortReport {
        let mut report = AbortReport {
            removed: Vec::new(),
            remove_failed: Vec::new(),
        };
        for staged in self.created {
            let media = MediaRef {
                owner: self.owner,
                id: staged.file.id,
            };
            if storage.delete(self.owner, staged.file.id).await.is_ok() {
                report.removed.push(media);
            } else {
                report.remove_failed.push(media);
            }
        }
        report
    }

    /// Keeps the staged files and deletes the previous ones.
    ///
    /// Call this only after the database transaction has committed. A delete
    /// failure does not undo that transaction.
    pub async fn commit<S: Storage + ?Sized>(self, storage: &S) -> CommitReport {
        let mut report = CommitReport {
            bound: self.kept,
            removed: Vec::new(),
            removal_failed: Vec::new(),
            retain_failed: Vec::new(),
        };
        for staged in &self.created {
            let media = MediaRef {
                owner: self.owner,
                id: staged.file.id,
            };
            if storage.retain(self.owner, staged.file.id).await.is_err() {
                report.retain_failed.push(media);
            }
            report.bound.push(BoundMedia {
                field: staged.field.clone(),
                media: Some(media),
            });
        }
        for media in self.removals {
            if storage.delete(media.owner, media.id).await.is_ok() {
                report.removed.push(media);
            } else {
                report.removal_failed.push(media);
            }
        }
        report
    }
}

pub fn prepare_media(
    actor: Uuid,
    owner: Uuid,
    access: &impl StorageAccess,
    policy: &MediaPolicy,
    fields: &[FieldSpec<'_>],
) -> Result<UploadPlan, Vec<FieldError>> {
    let mut errors = Vec::new();
    if access.permit(actor, owner, StorageAction::Upload).is_err() {
        errors.push(FieldError {
            field: fields
                .first()
                .map(|field| field.name)
                .unwrap_or("")
                .to_owned(),
            code: "validation.upload",
        });
        return Err(errors);
    }
    let mut names = Vec::new();
    let mut filenames = Vec::new();
    let mut total = 0u64;
    let mut decisions = Vec::new();
    let mut deletes = false;
    for field in fields {
        if names.iter().any(|seen| seen == &field.name) {
            errors.push(FieldError {
                field: field.name.to_owned(),
                code: "validation.duplicate",
            });
            continue;
        }
        names.push(field.name);
        if !field_name(field.name) {
            errors.push(FieldError {
                field: field.name.to_owned(),
                code: "validation.unknown_field",
            });
            continue;
        }
        if let Some(existing) = field.existing {
            if existing.owner != owner {
                errors.push(FieldError {
                    field: field.name.to_owned(),
                    code: "validation.upload",
                });
                continue;
            }
        }
        let incoming = field
            .incoming
            .as_ref()
            .filter(|file| !file.bytes.is_empty());
        if field.readonly && (incoming.is_some() || field.clear) {
            errors.push(FieldError {
                field: field.name.to_owned(),
                code: "validation.readonly",
            });
            continue;
        }
        if field.clear && incoming.is_none() {
            match field.existing {
                Some(media) if !field.required => {
                    deletes = true;
                    decisions.push(Decision::Remove {
                        field: field.name.to_owned(),
                        media,
                    });
                }
                Some(_) => {
                    errors.push(FieldError {
                        field: field.name.to_owned(),
                        code: "validation.required",
                    });
                }
                None if field.required => {
                    errors.push(FieldError {
                        field: field.name.to_owned(),
                        code: "validation.required",
                    });
                }
                None => decisions.push(Decision::Absent {
                    field: field.name.to_owned(),
                }),
            }
            continue;
        }
        let Some(incoming) = incoming else {
            if let Some(media) = field.existing {
                decisions.push(Decision::Keep {
                    field: field.name.to_owned(),
                    media,
                });
            } else if field.required {
                errors.push(FieldError {
                    field: field.name.to_owned(),
                    code: "validation.required",
                });
            } else {
                decisions.push(Decision::Absent {
                    field: field.name.to_owned(),
                });
            }
            continue;
        };
        if let Some(code) = name_code(incoming.original_name) {
            errors.push(FieldError {
                field: field.name.to_owned(),
                code,
            });
            continue;
        }
        let lowered = incoming.original_name.to_ascii_lowercase();
        if filenames.iter().any(|seen: &String| seen == &lowered) {
            errors.push(FieldError {
                field: field.name.to_owned(),
                code: "validation.duplicate",
            });
            continue;
        }
        filenames.push(lowered);
        let size = incoming.bytes.len() as u64;
        if size > policy.max_bytes || total.saturating_add(size) > policy.max_request_bytes {
            errors.push(FieldError {
                field: field.name.to_owned(),
                code: "validation.upload_size",
            });
            continue;
        }
        total = total.saturating_add(size);
        let Some(detected) = detect_type(incoming.bytes) else {
            errors.push(FieldError {
                field: field.name.to_owned(),
                code: "validation.upload_type",
            });
            continue;
        };
        if incoming
            .declared_type
            .is_some_and(|declared| declared != detected)
        {
            errors.push(FieldError {
                field: field.name.to_owned(),
                code: "validation.upload_type",
            });
            continue;
        }
        if !extension_matches(incoming.original_name, detected) {
            errors.push(FieldError {
                field: field.name.to_owned(),
                code: "validation.upload_type",
            });
            continue;
        }
        let pixels_decoded = match field.kind {
            MediaKind::File => {
                if !policy.file_types.contains(&detected) {
                    errors.push(FieldError {
                        field: field.name.to_owned(),
                        code: "validation.upload_type",
                    });
                    continue;
                }
                false
            }
            MediaKind::Image => match validate_image(incoming.bytes, policy.image_limits) {
                Ok(ImageValidation::PngDecoded(_)) => true,
                Ok(ImageValidation::JpegContainer(_)) => false,
                Err(StorageError::TooLarge) => {
                    errors.push(FieldError {
                        field: field.name.to_owned(),
                        code: "validation.upload_size",
                    });
                    continue;
                }
                Err(_) => {
                    errors.push(FieldError {
                        field: field.name.to_owned(),
                        code: "validation.upload_type",
                    });
                    continue;
                }
            },
        };
        if field.existing.is_some() {
            deletes = true;
        }
        decisions.push(Decision::Stage {
            field: field.name.to_owned(),
            upload: PreparedFile {
                original_name: incoming.original_name.to_owned(),
                content_type: detected.to_owned(),
                bytes: incoming.bytes.to_vec(),
                pixels_decoded,
            },
            retire: field.existing,
        });
    }
    if deletes && access.permit(actor, owner, StorageAction::Delete).is_err() {
        errors.push(FieldError {
            field: String::new(),
            code: "validation.upload",
        });
    }
    if errors.is_empty() {
        Ok(UploadPlan { owner, decisions })
    } else {
        Err(errors)
    }
}

fn field_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .chars()
            .all(|char| char.is_ascii_lowercase() || char.is_ascii_digit() || char == '_')
}

fn name_code(name: &str) -> Option<&'static str> {
    if !(1..=80).contains(&name.len())
        || !name
            .chars()
            .all(|char| char.is_ascii_alphanumeric() || matches!(char, '.' | '-' | '_'))
        || name.contains("..")
        || name.starts_with('.')
        || name.ends_with('.')
        || name.split('.').count() < 2
    {
        Some("validation.upload_name")
    } else {
        None
    }
}

fn extension_matches(name: &str, content_type: &str) -> bool {
    let Some(extension) = name.rsplit('.').next() else {
        return false;
    };
    let expected = match extension.to_ascii_lowercase().as_str() {
        "txt" => "text/plain",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        _ => return false,
    };
    expected == content_type
}

#[cfg(test)]
mod tests {
    use super::{
        prepare_media, FieldSpec, IncomingFile, MediaKind, MediaPolicy, MediaRef, PlannedField,
    };
    use crate::{ImageLimits, LocalStorage, OwnerAccess, Storage, StorageError};
    use async_trait::async_trait;
    use fexu_db::driver::{ConnectionTrait, TransactionTrait};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use uuid::Uuid;

    fn policy() -> MediaPolicy {
        MediaPolicy {
            max_bytes: 64_000,
            max_request_bytes: 80_000,
            file_types: &["text/plain", "application/pdf"],
            image_limits: ImageLimits {
                max_bytes: 64_000,
                max_width: 8,
                max_height: 8,
                max_pixels: 64,
            },
        }
    }

    fn root() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("fexu-media-{}", Uuid::new_v4()))
    }

    #[test]
    fn names_types_and_permissions_fail_before_any_write() {
        let owner = Uuid::new_v4();
        let other = Uuid::new_v4();
        let png = crate::png::sample_png(1, 1);
        let denied = prepare_media(
            other,
            owner,
            &OwnerAccess,
            &policy(),
            &[FieldSpec {
                name: "photo",
                kind: MediaKind::Image,
                required: true,
                readonly: false,
                existing: None,
                clear: false,
                incoming: Some(IncomingFile {
                    original_name: "photo.png",
                    declared_type: None,
                    bytes: &png,
                }),
            }],
        );
        assert_eq!(denied.expect_err("actor")[0].code, "validation.upload");
        let cases = [
            ("../photo.png", "validation.upload_name"),
            ("photo\\x.png", "validation.upload_name"),
            ("café.png", "validation.upload_name"),
            ("photo\u{202e}.png", "validation.upload_name"),
            (".hidden.png", "validation.upload_name"),
            ("photo.png.", "validation.upload_name"),
            ("", "validation.upload_name"),
        ];
        for (name, code) in cases {
            let errors = prepare_media(
                owner,
                owner,
                &OwnerAccess,
                &policy(),
                &[FieldSpec {
                    name: "photo",
                    kind: MediaKind::Image,
                    required: true,
                    readonly: false,
                    existing: None,
                    clear: false,
                    incoming: Some(IncomingFile {
                        original_name: name,
                        declared_type: None,
                        bytes: &png,
                    }),
                }],
            )
            .expect_err(name);
            assert_eq!(errors[0].code, code, "{name}");
        }
        let empty = prepare_media(
            owner,
            owner,
            &OwnerAccess,
            &policy(),
            &[FieldSpec {
                name: "notes",
                kind: MediaKind::File,
                required: true,
                readonly: false,
                existing: None,
                clear: false,
                incoming: Some(IncomingFile {
                    original_name: "notes.txt",
                    declared_type: Some("text/plain"),
                    bytes: b"",
                }),
            }],
        )
        .expect_err("empty");
        assert_eq!(empty[0].code, "validation.required");
        let forged = prepare_media(
            owner,
            owner,
            &OwnerAccess,
            &policy(),
            &[FieldSpec {
                name: "notes",
                kind: MediaKind::File,
                required: true,
                readonly: false,
                existing: None,
                clear: false,
                incoming: Some(IncomingFile {
                    original_name: "notes.txt",
                    declared_type: Some("image/png"),
                    bytes: b"hello",
                }),
            }],
        )
        .expect_err("mime");
        assert_eq!(forged[0].code, "validation.upload_type");
        let header = b"\x89PNG\r\n\x1a\nnot-a-png";
        let truncated = prepare_media(
            owner,
            owner,
            &OwnerAccess,
            &policy(),
            &[FieldSpec {
                name: "photo",
                kind: MediaKind::Image,
                required: true,
                readonly: false,
                existing: None,
                clear: false,
                incoming: Some(IncomingFile {
                    original_name: "photo.png",
                    declared_type: Some("image/png"),
                    bytes: header,
                }),
            }],
        )
        .expect_err("truncated");
        assert_eq!(truncated[0].code, "validation.upload_type");
    }

    #[tokio::test]
    async fn validation_failure_keeps_the_previous_file() {
        let directory = root();
        let storage = LocalStorage::open(&directory, 64_000, &["text/plain", "image/png"]).unwrap();
        let owner = Uuid::new_v4();
        let previous = storage
            .save(owner, "notes.txt", b"hello")
            .await
            .expect("previous");
        let existing = MediaRef {
            owner,
            id: previous.id,
        };
        let rejected = prepare_media(
            owner,
            owner,
            &OwnerAccess,
            &policy(),
            &[FieldSpec {
                name: "notes",
                kind: MediaKind::File,
                required: true,
                readonly: true,
                existing: Some(existing),
                clear: true,
                incoming: Some(IncomingFile {
                    original_name: "other.txt",
                    declared_type: None,
                    bytes: b"replaced",
                }),
            }],
        );
        assert_eq!(
            rejected.expect_err("readonly")[0].code,
            "validation.readonly"
        );
        let (_, bytes) = storage.read(owner, previous.id).await.expect("kept");
        assert_eq!(bytes, b"hello");
        let _ = std::fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn abort_after_a_failed_commit_removes_only_the_staged_file() {
        let directory = root();
        let storage = LocalStorage::open(&directory, 64_000, &["text/plain", "image/png"]).unwrap();
        let owner = Uuid::new_v4();
        let previous = storage.save(owner, "notes.txt", b"hello").await.unwrap();
        let png = crate::png::sample_png(1, 1);
        let plan = prepare_media(
            owner,
            owner,
            &OwnerAccess,
            &policy(),
            &[FieldSpec {
                name: "photo",
                kind: MediaKind::Image,
                required: true,
                readonly: false,
                existing: Some(MediaRef {
                    owner,
                    id: previous.id,
                }),
                clear: false,
                incoming: Some(IncomingFile {
                    original_name: "photo.png",
                    declared_type: Some("image/png"),
                    bytes: &png,
                }),
            }],
        )
        .expect("plan");
        assert!(plan.fields().any(|field| matches!(
            field,
            PlannedField::Stage {
                pixels_decoded: true,
                ..
            }
        )));
        let staged = plan.stage(&storage, 1_000, 60).await.expect("stage");
        let staged_id = staged.created()[0].file.id;
        assert!(storage
            .metadata(owner, staged_id)
            .await
            .unwrap()
            .expires_at
            .is_some());
        let db_committed = false;
        if db_committed {
            let _ = staged.commit(&storage).await;
        } else {
            let report = staged.abort(&storage).await;
            assert_eq!(report.remove_failed.len(), 0);
        }
        assert!(storage.read(owner, staged_id).await.is_err());
        let (_, bytes) = storage.read(owner, previous.id).await.unwrap();
        assert_eq!(bytes, b"hello");
        let _ = std::fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn database_rollback_drops_the_reference_and_abort_drops_the_bytes() {
        let directory = root();
        let storage = LocalStorage::open(&directory, 64_000, &["text/plain"]).unwrap();
        let owner = Uuid::new_v4();
        let previous = storage.save(owner, "notes.txt", b"hello").await.unwrap();
        let plan = prepare_media(
            owner,
            owner,
            &OwnerAccess,
            &policy(),
            &[FieldSpec {
                name: "notes",
                kind: MediaKind::File,
                required: true,
                readonly: false,
                existing: Some(MediaRef {
                    owner,
                    id: previous.id,
                }),
                clear: false,
                incoming: Some(IncomingFile {
                    original_name: "notes.txt",
                    declared_type: None,
                    bytes: b"updated",
                }),
            }],
        )
        .unwrap();
        let staged = plan.stage(&storage, 1_000, 60).await.unwrap();
        let staged_id = staged.created()[0].file.id;
        let db = fexu_db::connect("sqlite::memory:").await.expect("sqlite");
        db.execute(fexu_db::driver::Statement::from_string(
            fexu_db::driver::DatabaseBackend::Sqlite,
            "CREATE TABLE fexu_media_stage (id TEXT PRIMARY KEY NOT NULL)",
        ))
        .await
        .unwrap();
        let tx = db.begin().await.unwrap();
        tx.execute(fexu_db::driver::Statement::from_string(
            fexu_db::driver::DatabaseBackend::Sqlite,
            format!("INSERT INTO fexu_media_stage (id) VALUES ('{staged_id}')"),
        ))
        .await
        .unwrap();
        tx.rollback().await.unwrap();
        let rows = db
            .query_all(fexu_db::driver::Statement::from_string(
                fexu_db::driver::DatabaseBackend::Sqlite,
                "SELECT id FROM fexu_media_stage",
            ))
            .await
            .unwrap();
        assert!(rows.is_empty());
        staged.abort(&storage).await;
        assert!(matches!(
            storage.read(owner, staged_id).await,
            Err(StorageError::NotFound)
        ));
        assert!(storage.read(owner, previous.id).await.is_ok());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn committed_replacement_retains_the_new_file_and_deletes_the_old_one() {
        let directory = root();
        let storage = LocalStorage::open(&directory, 64_000, &["text/plain"]).unwrap();
        let owner = Uuid::new_v4();
        let previous = storage.save(owner, "notes.txt", b"hello").await.unwrap();
        let plan = prepare_media(
            owner,
            owner,
            &OwnerAccess,
            &policy(),
            &[FieldSpec {
                name: "notes",
                kind: MediaKind::File,
                required: false,
                readonly: false,
                existing: Some(MediaRef {
                    owner,
                    id: previous.id,
                }),
                clear: false,
                incoming: Some(IncomingFile {
                    original_name: "notes.txt",
                    declared_type: Some("text/plain"),
                    bytes: b"updated",
                }),
            }],
        )
        .unwrap();
        let staged = plan.stage(&storage, 1_000, 60).await.unwrap();
        let staged_id = staged.created()[0].file.id;
        let db = fexu_db::connect("sqlite::memory:").await.unwrap();
        db.execute(fexu_db::driver::Statement::from_string(
            fexu_db::driver::DatabaseBackend::Sqlite,
            "CREATE TABLE fexu_media_stage (id TEXT PRIMARY KEY NOT NULL)",
        ))
        .await
        .unwrap();
        let tx = db.begin().await.unwrap();
        tx.execute(fexu_db::driver::Statement::from_string(
            fexu_db::driver::DatabaseBackend::Sqlite,
            format!("INSERT INTO fexu_media_stage (id) VALUES ('{staged_id}')"),
        ))
        .await
        .unwrap();
        tx.commit().await.unwrap();
        let report = staged.commit(&storage).await;
        assert!(report.removal_failed.is_empty());
        assert!(report.retain_failed.is_empty());
        let meta = storage.metadata(owner, staged_id).await.unwrap();
        assert!(meta.expires_at.is_none());
        let (_, bytes) = storage.read(owner, staged_id).await.unwrap();
        assert_eq!(bytes, b"updated");
        assert!(storage.read(owner, previous.id).await.is_err());
        let rows = db
            .query_all(fexu_db::driver::Statement::from_string(
                fexu_db::driver::DatabaseBackend::Sqlite,
                "SELECT id FROM fexu_media_stage",
            ))
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn an_unreferenced_staged_file_expires() {
        let directory = root();
        let storage = LocalStorage::open(&directory, 64_000, &["text/plain"]).unwrap();
        let owner = Uuid::new_v4();
        let plan = prepare_media(
            owner,
            owner,
            &OwnerAccess,
            &policy(),
            &[FieldSpec {
                name: "notes",
                kind: MediaKind::File,
                required: false,
                readonly: false,
                existing: None,
                clear: false,
                incoming: Some(IncomingFile {
                    original_name: "notes.txt",
                    declared_type: None,
                    bytes: b"temp",
                }),
            }],
        )
        .unwrap();
        let staged = plan.stage(&storage, 100, 10).await.unwrap();
        let id = staged.created()[0].file.id;
        drop(staged);
        assert_eq!(storage.cleanup_orphans(109).unwrap().removed_expired, 0);
        assert!(storage.metadata(owner, id).await.is_ok());
        assert!(storage.cleanup_orphans(110).unwrap().removed_expired >= 1);
        assert!(storage.metadata(owner, id).await.is_err());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn a_write_failure_removes_the_file_written_earlier_in_the_batch() {
        let directory = root();
        let inner = LocalStorage::open(&directory, 64_000, &["text/plain"]).unwrap();
        let storage = Flaky {
            inner,
            remaining: AtomicUsize::new(1),
        };
        let owner = Uuid::new_v4();
        let plan = prepare_media(
            owner,
            owner,
            &OwnerAccess,
            &policy(),
            &[
                FieldSpec {
                    name: "one",
                    kind: MediaKind::File,
                    required: true,
                    readonly: false,
                    existing: None,
                    clear: false,
                    incoming: Some(IncomingFile {
                        original_name: "one.txt",
                        declared_type: None,
                        bytes: b"one",
                    }),
                },
                FieldSpec {
                    name: "two",
                    kind: MediaKind::File,
                    required: true,
                    readonly: false,
                    existing: None,
                    clear: false,
                    incoming: Some(IncomingFile {
                        original_name: "two.txt",
                        declared_type: None,
                        bytes: b"two",
                    }),
                },
            ],
        )
        .unwrap();
        assert!(matches!(
            plan.stage(&storage, 100, 30).await,
            Err(StorageError::Failed)
        ));
        let entries = std::fs::read_dir(directory.join(owner.to_string())).unwrap();
        let files = entries.filter_map(|entry| entry.ok()).count();
        assert_eq!(files, 0);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn parallel_stages_keep_distinct_ids() {
        let directory = root();
        let storage = Arc::new(LocalStorage::open(&directory, 64_000, &["text/plain"]).unwrap());
        let owner = Uuid::new_v4();
        let left_storage = storage.clone();
        let right_storage = storage.clone();
        let left = async move {
            let plan = prepare_media(
                owner,
                owner,
                &OwnerAccess,
                &policy(),
                &[FieldSpec {
                    name: "left",
                    kind: MediaKind::File,
                    required: true,
                    readonly: false,
                    existing: None,
                    clear: false,
                    incoming: Some(IncomingFile {
                        original_name: "left.txt",
                        declared_type: None,
                        bytes: b"left",
                    }),
                }],
            )
            .unwrap();
            plan.stage(left_storage.as_ref(), 100, 30).await.unwrap()
        };
        let right = async move {
            let plan = prepare_media(
                owner,
                owner,
                &OwnerAccess,
                &policy(),
                &[FieldSpec {
                    name: "right",
                    kind: MediaKind::File,
                    required: true,
                    readonly: false,
                    existing: None,
                    clear: false,
                    incoming: Some(IncomingFile {
                        original_name: "right.txt",
                        declared_type: None,
                        bytes: b"right",
                    }),
                }],
            )
            .unwrap();
            plan.stage(right_storage.as_ref(), 100, 30).await.unwrap()
        };
        let (left, right) = tokio::join!(left, right);
        assert_ne!(left.created()[0].file.id, right.created()[0].file.id);
        left.commit(storage.as_ref()).await;
        right.commit(storage.as_ref()).await;
        let _ = std::fs::remove_dir_all(directory);
    }

    struct Flaky {
        inner: LocalStorage,
        remaining: AtomicUsize,
    }

    #[async_trait]
    impl Storage for Flaky {
        async fn save(
            &self,
            owner: Uuid,
            original_name: &str,
            bytes: &[u8],
        ) -> Result<crate::StoredFile, StorageError> {
            self.save_expiring(owner, original_name, bytes, None).await
        }
        async fn read(
            &self,
            owner: Uuid,
            id: Uuid,
        ) -> Result<(crate::StoredFile, Vec<u8>), StorageError> {
            self.inner.read(owner, id).await
        }
        async fn delete(&self, owner: Uuid, id: Uuid) -> Result<(), StorageError> {
            self.inner.delete(owner, id).await
        }
        async fn exists(&self, owner: Uuid, id: Uuid) -> Result<bool, StorageError> {
            self.inner.exists(owner, id).await
        }
        async fn metadata(&self, owner: Uuid, id: Uuid) -> Result<crate::StoredFile, StorageError> {
            self.inner.metadata(owner, id).await
        }
        async fn save_expiring(
            &self,
            owner: Uuid,
            original_name: &str,
            bytes: &[u8],
            expires_at: Option<i64>,
        ) -> Result<crate::StoredFile, StorageError> {
            if self.remaining.fetch_sub(1, Ordering::SeqCst) == 0 {
                return Err(StorageError::Failed);
            }
            self.inner
                .save_expiring(owner, original_name, bytes, expires_at)
                .await
        }
    }

    async fn database_roundtrip(url: &str, backend: fexu_db::driver::DatabaseBackend) {
        let directory = root();
        let storage = LocalStorage::open(&directory, 64_000, &["text/plain"]).unwrap();
        let owner = Uuid::new_v4();
        let plan = prepare_media(
            owner,
            owner,
            &OwnerAccess,
            &policy(),
            &[FieldSpec {
                name: "notes",
                kind: MediaKind::File,
                required: true,
                readonly: false,
                existing: None,
                clear: false,
                incoming: Some(IncomingFile {
                    original_name: "notes.txt",
                    declared_type: None,
                    bytes: b"stored",
                }),
            }],
        )
        .unwrap();
        let staged = plan.stage(&storage, 100, 60).await.unwrap();
        let id = staged.created()[0].file.id;
        let db = fexu_db::connect(url).await.expect("database");
        let drop_sql = "DROP TABLE IF EXISTS fexu_media_stage";
        let create_sql = "CREATE TABLE fexu_media_stage (id TEXT PRIMARY KEY NOT NULL)";
        db.execute(fexu_db::driver::Statement::from_string(backend, drop_sql))
            .await
            .unwrap();
        db.execute(fexu_db::driver::Statement::from_string(backend, create_sql))
            .await
            .unwrap();
        let tx = db.begin().await.unwrap();
        tx.execute(fexu_db::driver::Statement::from_string(
            backend,
            format!("INSERT INTO fexu_media_stage (id) VALUES ('{id}')"),
        ))
        .await
        .unwrap();
        tx.rollback().await.unwrap();
        staged.abort(&storage).await;
        assert!(storage.read(owner, id).await.is_err());
        db.execute(fexu_db::driver::Statement::from_string(backend, drop_sql))
            .await
            .unwrap();
        let _ = std::fs::remove_dir_all(directory);
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL for a disposable PostgreSQL database"]
    async fn postgres_rollback_removes_the_staged_file_only_through_abort() {
        let url = std::env::var("DATABASE_URL").unwrap_or_default();
        assert!(
            url.starts_with("postgres"),
            "DATABASE_URL must be PostgreSQL"
        );
        database_roundtrip(&url, fexu_db::driver::DatabaseBackend::Postgres).await;
    }
}
