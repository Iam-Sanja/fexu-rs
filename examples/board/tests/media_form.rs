use fexu::forms::{bind_media, MediaField};
use fexu::storage::{ImageLimits, MediaPolicy, OwnerAccess, PlannedField};
use fexu::uuid::Uuid;

use board::{note_cover_field, note_cover_html};

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

#[test]
fn cover_form_rejects_a_missing_file_and_keeps_the_error_escaped() {
    let owner = Uuid::new_v4();
    let missing = bind_media(
        owner,
        owner,
        &OwnerAccess,
        &policy(),
        &[note_cover_field(None)],
    )
    .unwrap_err();
    assert_eq!(missing.for_field("cover"), Some("validation.required"));
    let html = note_cover_html(Some("<script>"), Some("<cover>.txt"));
    assert!(html.contains("type=\"file\""));
    assert!(html.contains("&lt;script&gt;"));
    assert!(html.contains("&lt;cover&gt;.txt"));
    assert!(!html.contains("<script>"));

    let wrong = bind_media(
        owner,
        owner,
        &OwnerAccess,
        &policy(),
        &[note_cover_field(Some(("notes.txt", b"\x89PNG")))],
    )
    .unwrap_err();
    assert_eq!(wrong.for_field("cover"), Some("validation.upload_type"));

    let readonly = bind_media(
        owner,
        owner,
        &OwnerAccess,
        &policy(),
        &[MediaField::file("cover")
            .readonly()
            .upload("notes.txt", Some("text/plain"), b"hello")],
    )
    .unwrap_err();
    assert_eq!(readonly.for_field("cover"), Some("validation.readonly"));

    let accepted = bind_media(
        owner,
        owner,
        &OwnerAccess,
        &policy(),
        &[note_cover_field(Some(("notes.txt", b"hello")))],
    )
    .unwrap();
    let staged: Vec<_> = accepted.fields().collect();
    assert!(matches!(
        staged.as_slice(),
        [PlannedField::Stage {
            field: "cover",
            size: 5,
            ..
        }]
    ));
}
