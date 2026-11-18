//! Validates an admin record image with the storage field rules, then leaves
//! the bytes in `_fexu_record_files`.
//!
//! The row is written by `files::save_on` inside the record transaction.
//! `abort` and `commit` are not used here: a database rollback already removes
//! the blob. That is a different store from `LocalStorage`. The two commits
//! are not one transaction.

use fexu_storage::{
    prepare_media, upload_name_code, FieldSpec, ImageLimits, IncomingFile, MediaKind, MediaPolicy,
    OwnerAccess,
};
use uuid::Uuid;

#[derive(Debug)]
pub(crate) struct ReviewedUpload {
    pub label: String,
    pub kind: &'static str,
}

const IMAGE_LIMITS: ImageLimits = ImageLimits {
    max_bytes: crate::files::IMAGE_BYTES as u64,
    max_width: 4_000,
    max_height: 4_000,
    max_pixels: 8_000_000,
};

/// Checks one posted image. PNG and JPEG use the image decoder. GIF and WebP
/// stay signature checks so existing admin files keep their types. ICO is the
/// admin favicon exception and is not a storage image type.
pub(crate) fn review_record_upload(
    actor: Uuid,
    owner: Uuid,
    filename: &str,
    declared_type: Option<&str>,
    bytes: &[u8],
) -> Result<ReviewedUpload, &'static str> {
    if let Some(code) = upload_name_code(filename) {
        return Err(code);
    }
    let kind = crate::files::image_type(bytes).ok_or("validation.upload_type")?;
    if matches!(
        kind,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp"
    ) {
        let media_kind = if matches!(kind, "image/png" | "image/jpeg") {
            MediaKind::Image
        } else {
            MediaKind::File
        };
        let policy = MediaPolicy {
            max_bytes: crate::files::IMAGE_BYTES as u64,
            max_request_bytes: crate::files::IMAGE_BYTES as u64,
            file_types: &["image/png", "image/jpeg", "image/gif", "image/webp"],
            image_limits: IMAGE_LIMITS,
        };
        let field = FieldSpec {
            name: "image",
            kind: media_kind,
            required: true,
            readonly: false,
            existing: None,
            clear: false,
            image_limits: None,
            incoming: Some(IncomingFile {
                original_name: filename,
                declared_type,
                bytes,
            }),
        };
        prepare_media(actor, owner, &OwnerAccess, &policy, &[field]).map_err(|errors| {
            errors
                .first()
                .map(|error| error.code)
                .unwrap_or("validation.upload")
        })?;
        return Ok(ReviewedUpload {
            label: filename.to_owned(),
            kind,
        });
    }
    if kind != "image/x-icon" {
        return Err("validation.upload_type");
    }
    if bytes.len() as u64 > crate::files::IMAGE_BYTES as u64 {
        return Err("validation.upload_size");
    }
    if !filename.to_ascii_lowercase().ends_with(".ico") {
        return Err("validation.upload_type");
    }
    if declared_type.is_some_and(|declared| declared != "image/x-icon") {
        return Err("validation.upload_type");
    }
    if actor != owner {
        return Err("validation.upload");
    }
    Ok(ReviewedUpload {
        label: filename.to_owned(),
        kind,
    })
}

/// `Ok(true)` means the existing blob may be deleted inside the same transaction.
pub(crate) fn review_record_clear(
    allowed: bool,
    required: bool,
    has_bytes: bool,
    replacing: bool,
) -> Result<bool, &'static str> {
    if !allowed {
        return Err("validation.unknown_field");
    }
    if replacing || !has_bytes {
        return Ok(false);
    }
    if required {
        return Err("validation.required");
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::{review_record_clear, review_record_upload};
    use uuid::Uuid;

    #[test]
    fn record_images_reject_forged_names_and_a_truncated_png() {
        let owner = Uuid::new_v4();
        for name in ["../a.png", "<script>.png", "café.png"] {
            let error =
                review_record_upload(owner, owner, name, None, b"\x89PNG\r\n\x1a\n").unwrap_err();
            assert_eq!(error, "validation.upload_name", "{name}");
        }
        let truncated = review_record_upload(
            owner,
            owner,
            "photo.png",
            Some("image/png"),
            b"\x89PNG\r\n\x1a\ncut",
        )
        .unwrap_err();
        assert_eq!(truncated, "validation.upload_type");
        let declared =
            review_record_upload(owner, owner, "a.gif", Some("image/png"), b"GIF89a").unwrap_err();
        assert_eq!(declared, "validation.upload_type");
        let foreign =
            review_record_upload(Uuid::new_v4(), owner, "a.gif", None, b"GIF89a").unwrap_err();
        assert_eq!(foreign, "validation.upload");
        let gif = review_record_upload(owner, owner, "a.gif", None, b"GIF89a").unwrap();
        assert_eq!(gif.kind, "image/gif");
        assert_eq!(gif.label, "a.gif");
        let icon = review_record_upload(
            owner,
            owner,
            "icon.ico",
            Some("image/x-icon"),
            &[0, 0, 1, 0, 1, 0],
        )
        .unwrap();
        assert_eq!(icon.kind, "image/x-icon");
        assert_eq!(
            review_record_clear(true, true, true, false).unwrap_err(),
            "validation.required"
        );
        assert!(!review_record_clear(true, false, true, true).unwrap());
        assert!(review_record_clear(true, false, true, false).unwrap());
        assert_eq!(
            review_record_clear(false, false, true, false).unwrap_err(),
            "validation.unknown_field"
        );
        let english =
            crate::localization::validation_text("en", "field:cover: validation.upload_name");
        assert_eq!(english, "The file name is invalid.");
        let german =
            crate::localization::validation_text("de", "field:cover: validation.upload_size");
        assert_eq!(german, "Die Datei ist zu groß.");
    }
}
