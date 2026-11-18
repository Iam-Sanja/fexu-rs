//! Maps the storage field lifecycle onto form error codes.
//!
//! Widgets and app `forms.rs` modules call [`fexu_storage::prepare_media`]
//! before opening a database transaction. [`fexu_storage::UploadPlan::stage`]
//! writes new ids only. [`fexu_storage::StagedMedia::abort`] runs when
//! validation or the transaction fails. [`fexu_storage::StagedMedia::commit`]
//! runs after the transaction commits. This module does not store bytes.

use fexu_storage::FieldError;

use crate::FormErrors;

/// Copies storage field errors into [`FormErrors`] without changing the code.
pub fn media_field_errors(errors: Vec<FieldError>) -> FormErrors {
    let mut form = FormErrors::default();
    for error in errors {
        form.push(error.field, error.code);
    }
    form
}

#[cfg(test)]
mod tests {
    use super::media_field_errors;
    use fexu_storage::{
        prepare_media, FieldSpec, IncomingFile, MediaKind, MediaPolicy, OwnerAccess,
    };
    use uuid::Uuid;

    #[test]
    fn html_and_json_forms_share_the_storage_codes() {
        let owner = Uuid::new_v4();
        let errors = prepare_media(
            owner,
            owner,
            &OwnerAccess,
            &MediaPolicy {
                max_bytes: 32,
                max_request_bytes: 32,
                file_types: &["text/plain"],
                image_limits: fexu_storage::ImageLimits {
                    max_bytes: 32,
                    max_width: 1,
                    max_height: 1,
                    max_pixels: 1,
                },
            },
            &[
                FieldSpec {
                    name: "notes",
                    kind: MediaKind::File,
                    required: true,
                    readonly: false,
                    existing: None,
                    clear: false,
                    incoming: Some(IncomingFile {
                        original_name: "../notes.txt",
                        declared_type: None,
                        bytes: b"hello",
                    }),
                },
                FieldSpec {
                    name: "photo",
                    kind: MediaKind::Image,
                    required: true,
                    readonly: false,
                    existing: None,
                    clear: false,
                    incoming: Some(IncomingFile {
                        original_name: "photo.png",
                        declared_type: Some("image/png"),
                        bytes: b"\x89PNG\r\n\x1a\ncut",
                    }),
                },
            ],
        )
        .expect_err("invalid");
        let form = media_field_errors(errors);
        assert_eq!(form.for_field("notes"), Some("validation.upload_name"));
        assert_eq!(form.for_field("photo"), Some("validation.upload_type"));
    }
}
