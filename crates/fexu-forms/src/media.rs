//! File and image fields for an application `forms.rs`.
//!
//! [`bind_media`] checks every field and writes nothing. [`finish_media`]
//! stages new ids, polls the database future, and then commits or aborts the
//! storage batch. Those are two steps. A database rollback does not reach the
//! storage backend unless this function aborts the staged ids. Storage
//! `commit` cannot roll the database back, and a failed delete stays listed on
//! [`fexu_storage::CommitReport`].

use std::future::Future;

use fexu_storage::{
    prepare_media, read_multipart, AbortReport, CommitReport, FieldSpec, ImageLimits, IncomingFile,
    MediaKind, MediaPolicy, MediaRef, MultipartError, Storage, StorageAccess, StorageError,
    UploadPlan, MULTIPART_BODY_LIMIT,
};
use uuid::Uuid;

use crate::{escape, media_field_errors, FormErrors};

/// One file or image field. Build it with [`MediaField::file`] or [`MediaField::image`].
pub struct MediaField<'a> {
    name: &'a str,
    kind: MediaKind,
    required: bool,
    readonly: bool,
    existing: Option<MediaRef>,
    clear: bool,
    limits: Option<ImageLimits>,
    incoming: Option<IncomingFile<'a>>,
}

impl<'a> MediaField<'a> {
    pub fn file(name: &'a str) -> Self {
        Self::new(name, MediaKind::File)
    }

    pub fn image(name: &'a str) -> Self {
        Self::new(name, MediaKind::Image)
    }

    fn new(name: &'a str, kind: MediaKind) -> Self {
        Self {
            name,
            kind,
            required: false,
            readonly: false,
            existing: None,
            clear: false,
            limits: None,
            incoming: None,
        }
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    pub fn readonly(mut self) -> Self {
        self.readonly = true;
        self
    }

    pub fn existing(mut self, media: MediaRef) -> Self {
        self.existing = Some(media);
        self
    }

    pub fn clear(mut self) -> Self {
        self.clear = true;
        self
    }

    /// Pixel limits for this image field. File fields ignore them.
    pub fn limits(mut self, limits: ImageLimits) -> Self {
        self.limits = Some(limits);
        self
    }

    pub fn upload(
        mut self,
        original_name: &'a str,
        declared_type: Option<&'a str>,
        bytes: &'a [u8],
    ) -> Self {
        self.incoming = Some(IncomingFile {
            original_name,
            declared_type,
            bytes,
        });
        self
    }
}

/// Validates the fields. No file is created or deleted.
pub fn bind_media(
    actor: Uuid,
    owner: Uuid,
    access: &impl StorageAccess,
    policy: &MediaPolicy,
    fields: &[MediaField<'_>],
) -> Result<UploadPlan, FormErrors> {
    let specs: Vec<FieldSpec<'_>> = fields
        .iter()
        .map(|field| FieldSpec {
            name: field.name,
            kind: field.kind,
            required: field.required,
            readonly: field.readonly,
            existing: field.existing,
            clear: field.clear,
            image_limits: field.limits,
            incoming: field.incoming.as_ref().map(|incoming| IncomingFile {
                original_name: incoming.original_name,
                declared_type: incoming.declared_type,
                bytes: incoming.bytes,
            }),
        })
        .collect();
    prepare_media(actor, owner, access, policy, &specs).map_err(media_field_errors)
}

/// A parsed multipart body. File bytes stay here until [`bind_media`] accepts them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaParts {
    pub text: Vec<(String, String)>,
    pub files: Vec<MediaPart>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaPart {
    pub field: String,
    pub original_name: String,
    pub declared_type: Option<String>,
    pub bytes: Vec<u8>,
}

impl MediaParts {
    pub fn text(&self, field: &str) -> Option<&str> {
        self.text
            .iter()
            .find(|(name, _)| name == field)
            .map(|(_, value)| value.as_str())
    }

    pub fn file(&self, field: &str) -> Option<&MediaPart> {
        self.files.iter().find(|file| file.field == field)
    }

    /// `{field}__clear` is `1` or `true`.
    pub fn cleared(&self, field: &str) -> bool {
        let name = format!("{field}__clear");
        matches!(self.text(&name), Some("1" | "true"))
    }
}

/// Reads `multipart/form-data` without writing a file.
///
/// `max_bytes` and [`MULTIPART_BODY_LIMIT`] both apply. Duplicate names are
/// `validation.duplicate`. A path-like filename is `validation.upload_name`.
pub fn read_media_parts(
    content_type: &str,
    body: &[u8],
    max_bytes: usize,
) -> Result<MediaParts, FormErrors> {
    if body.len() > max_bytes || body.len() > MULTIPART_BODY_LIMIT {
        let mut errors = FormErrors::default();
        errors.push_form("validation.upload_size");
        return Err(errors);
    }
    let parts = match read_multipart(content_type, body) {
        Ok(parts) => parts,
        Err(error) => return Err(multipart_error(error)),
    };
    let mut text = Vec::new();
    let mut files = Vec::new();
    for part in parts {
        if let Some(filename) = part.filename {
            files.push(MediaPart {
                field: part.name,
                original_name: filename,
                declared_type: part.declared_type,
                bytes: part.bytes,
            });
            continue;
        }
        let value = String::from_utf8(part.bytes).map_err(|_| {
            let mut errors = FormErrors::default();
            errors.push(&part.name, "validation.upload");
            errors
        })?;
        text.push((part.name, value));
    }
    Ok(MediaParts { text, files })
}

fn multipart_error(error: MultipartError) -> FormErrors {
    let mut errors = FormErrors::default();
    match error {
        MultipartError::TooLarge => errors.push_form("validation.upload_size"),
        MultipartError::DuplicateField(name) => errors.push(name, "validation.duplicate"),
        MultipartError::InvalidFilename(_) => errors.push_form("validation.upload_name"),
        MultipartError::Malformed => errors.push_form("validation.upload"),
    }
    errors
}

/// HTML for one media field. The current name is escaped and is not a path.
pub struct MediaView<'a> {
    pub name: &'a str,
    pub label: &'a str,
    pub image: bool,
    pub required: bool,
    pub readonly: bool,
    pub current_name: Option<&'a str>,
    pub error: Option<&'a str>,
}

pub fn render_media_field(view: MediaView<'_>) -> String {
    let name = escape(view.name);
    let label = escape(view.label);
    let mut html = format!("<label for=\"{name}\">{label}</label>");
    if let Some(current) = view.current_name {
        html.push_str(&format!("<p>{}</p>", escape(current)));
    }
    if !view.readonly {
        let accept = if view.image {
            " accept=\"image/png,image/jpeg\""
        } else {
            ""
        };
        let required = if view.required && view.current_name.is_none() {
            " required"
        } else {
            ""
        };
        html.push_str(&format!(
            "<input id=\"{name}\" name=\"{name}\" type=\"file\"{accept}{required}>"
        ));
        if view.current_name.is_some() && !view.required {
            html.push_str(&format!(
                "<input type=\"checkbox\" name=\"{name}__clear\" value=\"1\">"
            ));
        }
    }
    if let Some(error) = view.error {
        html.push_str(&format!("<p role=\"alert\">{}</p>", escape(error)));
    }
    html
}

/// Database save failed after staging. The staged ids were deleted when `abort` succeeded.
#[derive(Debug)]
pub enum MediaFinishError<E> {
    Stage(StorageError),
    Database { error: E, abort: AbortReport },
}

/// Stages, then polls `save`, then commits or aborts storage.
///
/// `save` must be lazy. Its work starts when this function polls it, after
/// staging has succeeded. A storage `commit` still cannot undo `save`.
pub async fn finish_media<S, T, E, F>(
    plan: UploadPlan,
    storage: &S,
    now: i64,
    ttl: i64,
    save: F,
) -> Result<(T, CommitReport), MediaFinishError<E>>
where
    S: Storage + ?Sized,
    F: Future<Output = Result<T, E>>,
{
    let staged = match plan.stage(storage, now, ttl).await {
        Ok(staged) => staged,
        Err(error) => return Err(MediaFinishError::Stage(error)),
    };
    match save.await {
        Ok(value) => Ok((value, staged.commit(storage).await)),
        Err(error) => Err(MediaFinishError::Database {
            error,
            abort: staged.abort(storage).await,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        bind_media, finish_media, read_media_parts, render_media_field, MediaField, MediaView,
    };
    use fexu_storage::{ImageLimits, LocalStorage, MediaPolicy, MediaRef, OwnerAccess, Storage};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use uuid::Uuid;

    fn policy() -> MediaPolicy {
        MediaPolicy {
            max_bytes: 64_000,
            max_request_bytes: 80_000,
            file_types: &["text/plain"],
            image_limits: ImageLimits {
                max_bytes: 64_000,
                max_width: 8,
                max_height: 8,
                max_pixels: 64,
            },
        }
    }

    fn tight() -> ImageLimits {
        ImageLimits {
            max_bytes: 32,
            max_width: 1,
            max_height: 1,
            max_pixels: 1,
        }
    }

    #[test]
    fn field_errors_cover_name_type_owner_and_readonly() {
        let owner = Uuid::new_v4();
        let other = Uuid::new_v4();
        let foreign = bind_media(
            other,
            owner,
            &OwnerAccess,
            &policy(),
            &[MediaField::file("notes")
                .required()
                .upload("notes.txt", None, b"hello")],
        );
        assert_eq!(
            foreign.unwrap_err().for_field("notes"),
            Some("validation.upload")
        );
        let names = [
            ("../notes.txt", "validation.upload_name"),
            ("<script>.txt", "validation.upload_name"),
            ("café.txt", "validation.upload_name"),
            ("notes.txt", "validation.upload_type"),
        ];
        for (name, code) in names {
            let bytes: &[u8] =
                if name.ends_with(".txt") && !name.contains('<') && !name.contains('/') {
                    b"\x89PNG"
                } else {
                    b"hello"
                };
            let errors = bind_media(
                owner,
                owner,
                &OwnerAccess,
                &policy(),
                &[MediaField::file("notes")
                    .required()
                    .upload(name, Some("text/plain"), bytes)],
            )
            .unwrap_err();
            assert_eq!(errors.for_field("notes"), Some(code), "{name}");
        }
        let readonly = bind_media(
            owner,
            owner,
            &OwnerAccess,
            &policy(),
            &[MediaField::file("notes")
                .readonly()
                .existing(MediaRef {
                    owner,
                    id: Uuid::new_v4(),
                })
                .upload("notes.txt", None, b"hello")],
        )
        .unwrap_err();
        assert_eq!(readonly.for_field("notes"), Some("validation.readonly"));
        let truncated = bind_media(
            owner,
            owner,
            &OwnerAccess,
            &policy(),
            &[MediaField::image("photo")
                .required()
                .limits(tight())
                .upload("photo.png", Some("image/png"), b"\x89PNG\r\n\x1a\ncut")],
        )
        .unwrap_err();
        assert_eq!(truncated.for_field("photo"), Some("validation.upload_type"));
    }

    #[test]
    fn multipart_rejects_duplicates_and_a_forged_type_stays_unbound() {
        let body = b"\
--bound\r\nContent-Disposition: form-data; name=\"notes\"; filename=\"notes.txt\"\r\nContent-Type: text/plain\r\n\r\nhello\r\n\
--bound\r\nContent-Disposition: form-data; name=\"notes\"; filename=\"other.txt\"\r\n\r\nagain\r\n\
--bound--\r\n";
        let duplicate = read_media_parts("multipart/form-data; boundary=bound", body, 8_000);
        assert_eq!(
            duplicate.unwrap_err().for_field("notes"),
            Some("validation.duplicate")
        );
        let once = b"\
--bound\r\nContent-Disposition: form-data; name=\"notes\"; filename=\"../notes.txt\"\r\nContent-Type: text/plain\r\n\r\nhello\r\n\
--bound\r\nContent-Disposition: form-data; name=\"notes__clear\"\r\n\r\n1\r\n\
--bound--\r\n";
        assert_eq!(
            read_media_parts("multipart/form-data; boundary=bound", once, 8_000)
                .unwrap_err()
                .form_error(),
            Some("validation.upload_name")
        );
        let html = render_media_field(MediaView {
            name: "notes",
            label: "Notes",
            image: false,
            required: false,
            readonly: false,
            current_name: Some("<script>"),
            error: Some("invalid"),
        });
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<script>"));
        assert!(html.contains("name=\"notes__clear\""));
    }

    #[tokio::test]
    async fn a_failed_database_save_keeps_the_previous_file() {
        let directory = std::env::temp_dir().join(format!("fexu-form-media-{}", Uuid::new_v4()));
        let storage = LocalStorage::open(&directory, 64_000, &["text/plain"]).unwrap();
        let owner = Uuid::new_v4();
        let previous = storage.save(owner, "notes.txt", b"hello").await.unwrap();
        let plan = bind_media(
            owner,
            owner,
            &OwnerAccess,
            &policy(),
            &[MediaField::file("notes")
                .required()
                .existing(MediaRef {
                    owner,
                    id: previous.id,
                })
                .upload("notes.txt", Some("text/plain"), b"updated")],
        )
        .unwrap();
        let started = Arc::new(AtomicBool::new(false));
        let flag = started.clone();
        let failed = finish_media(plan, &storage, 1_000, 60, async move {
            flag.store(true, Ordering::SeqCst);
            Err::<(), &str>("rolled back")
        })
        .await;
        assert!(started.load(Ordering::SeqCst));
        assert!(failed.is_err());
        let (_, bytes) = storage.read(owner, previous.id).await.unwrap();
        assert_eq!(bytes, b"hello");
        let kept = std::fs::read_dir(directory.join(owner.to_string()))
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| Uuid::parse_str(name).is_ok())
            })
            .count();
        assert_eq!(kept, 1);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn stage_failure_does_not_run_the_database_save() {
        let directory = std::env::temp_dir().join(format!("fexu-form-stage-{}", Uuid::new_v4()));
        let storage = LocalStorage::open(&directory, 4, &["text/plain"]).unwrap();
        let owner = Uuid::new_v4();
        let plan = bind_media(
            owner,
            owner,
            &OwnerAccess,
            &policy(),
            &[MediaField::file("notes")
                .required()
                .upload("notes.txt", None, b"hello")],
        )
        .unwrap();
        let started = Arc::new(AtomicBool::new(false));
        let flag = started.clone();
        let failed = finish_media(plan, &storage, 1_000, 60, async move {
            flag.store(true, Ordering::SeqCst);
            Ok::<(), ()>(())
        })
        .await;
        assert!(failed.is_err());
        assert!(!started.load(Ordering::SeqCst));
        let _ = std::fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn sqlite_rollback_and_commit_follow_the_storage_contract() {
        roundtrip("sqlite::memory:", fexu_db::driver::DatabaseBackend::Sqlite).await;
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL for a disposable PostgreSQL database"]
    async fn postgres_media_form_abort_follows_the_database() {
        let url = std::env::var("DATABASE_URL").unwrap_or_default();
        assert!(
            url.starts_with("postgres"),
            "DATABASE_URL must be PostgreSQL"
        );
        roundtrip(&url, fexu_db::driver::DatabaseBackend::Postgres).await;
    }

    async fn roundtrip(url: &str, backend: fexu_db::driver::DatabaseBackend) {
        use fexu_db::driver::{ConnectionTrait, TransactionTrait};
        let directory = std::env::temp_dir().join(format!("fexu-form-db-{}", Uuid::new_v4()));
        let storage = LocalStorage::open(&directory, 64_000, &["text/plain"]).unwrap();
        let owner = Uuid::new_v4();
        let previous = storage.save(owner, "notes.txt", b"hello").await.unwrap();
        let plan = bind_media(
            owner,
            owner,
            &OwnerAccess,
            &policy(),
            &[MediaField::file("notes")
                .existing(MediaRef {
                    owner,
                    id: previous.id,
                })
                .upload("notes.txt", None, b"updated")],
        )
        .unwrap();
        let db = fexu_db::connect(url).await.expect("database");
        db.execute(fexu_db::driver::Statement::from_string(
            backend,
            "DROP TABLE IF EXISTS fexu_form_media",
        ))
        .await
        .unwrap();
        db.execute(fexu_db::driver::Statement::from_string(
            backend,
            "CREATE TABLE fexu_form_media (id TEXT PRIMARY KEY NOT NULL)",
        ))
        .await
        .unwrap();
        let failed = finish_media(plan, &storage, 1_000, 60, async {
            let tx = db.begin().await.unwrap();
            tx.execute(fexu_db::driver::Statement::from_string(
                backend,
                "INSERT INTO fexu_form_media (id) VALUES ('new')",
            ))
            .await
            .unwrap();
            tx.rollback().await.unwrap();
            Err::<(), &str>("rolled back")
        })
        .await;
        assert!(failed.is_err());
        assert!(storage.read(owner, previous.id).await.is_ok());
        let rows = db
            .query_all(fexu_db::driver::Statement::from_string(
                backend,
                "SELECT id FROM fexu_form_media",
            ))
            .await
            .unwrap();
        assert!(rows.is_empty());
        let plan = bind_media(
            owner,
            owner,
            &OwnerAccess,
            &policy(),
            &[MediaField::file("notes")
                .existing(MediaRef {
                    owner,
                    id: previous.id,
                })
                .upload("notes.txt", None, b"stored")],
        )
        .unwrap();
        let ((), report) = finish_media(plan, &storage, 1_000, 60, async {
            let tx = db.begin().await.unwrap();
            tx.execute(fexu_db::driver::Statement::from_string(
                backend,
                "INSERT INTO fexu_form_media (id) VALUES ('kept')",
            ))
            .await
            .unwrap();
            tx.commit().await.unwrap();
            Ok::<(), &str>(())
        })
        .await
        .expect("commit");
        assert!(report.removal_failed.is_empty());
        assert!(storage.read(owner, previous.id).await.is_err());
        let bound = report
            .bound
            .iter()
            .find(|item| item.field == "notes")
            .and_then(|item| item.media)
            .expect("bound");
        let (_, bytes) = storage.read(owner, bound.id).await.unwrap();
        assert_eq!(bytes, b"stored");
        db.execute(fexu_db::driver::Statement::from_string(
            backend,
            "DROP TABLE IF EXISTS fexu_form_media",
        ))
        .await
        .unwrap();
        let _ = std::fs::remove_dir_all(directory);
    }
}
