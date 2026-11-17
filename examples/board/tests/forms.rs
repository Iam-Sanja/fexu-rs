use fexu::core::{QueryScope, ScopeValue};
use fexu::db::{ModelRepository, PatchField};
use fexu::forms::{FexuForm, FormValues};
use fexu::prelude::*;

use board::{prepare_database, Note, NotePublishForm, NoteReviseForm, NoteSearchForm};

async fn publish_and_revise(db: fexu::db::driver::DatabaseConnection) {
    prepare_database(&db).await.expect("schema");
    let owner = Uuid::new_v4();
    let mut posted = FormValues::default();
    posted.insert("title", "Visible");
    posted.insert("body", "Hello");
    posted.insert("owner_id", owner.to_string());
    assert_eq!(
        NotePublishForm::from_form(&posted)
            .unwrap_err()
            .for_field("owner_id"),
        Some("validation.unknown_field")
    );
    posted.remove("owner_id");
    let publish = NotePublishForm::from_form(&posted).unwrap();
    let created = Note::create(
        &db,
        board::NoteCreate {
            title: publish.title,
            body: publish.body,
            owner_id: owner,
        },
    )
    .await
    .expect("create");
    assert_eq!(created.owner_id, owner);

    let mut forged = FormValues::default();
    forged.insert("title", "Renamed");
    forged.insert("owner_id", Uuid::new_v4().to_string());
    assert_eq!(
        NoteReviseForm::from_submit(fexu::forms::SubmitMode::Patch, &forged)
            .unwrap_err()
            .for_field("owner_id"),
        Some("validation.unknown_field")
    );
    forged.remove("owner_id");
    let revised = NoteReviseForm::from_submit(fexu::forms::SubmitMode::Patch, &forged).unwrap();
    let scope = QueryScope::Equals {
        column: "owner_id",
        value: ScopeValue::Uuid(owner),
    };
    let updated = Note::update_scoped(
        &db,
        created.id,
        board::NoteUpdate {
            title: PatchField::Value(revised.title),
            body: PatchField::Missing,
            owner_id: PatchField::Missing,
        },
        &scope,
    )
    .await
    .expect("update");
    assert_eq!(updated.title, "Renamed");
    assert_eq!(updated.owner_id, owner);
    let foreign = QueryScope::Equals {
        column: "owner_id",
        value: ScopeValue::Uuid(Uuid::new_v4()),
    };
    let hidden = Note::update_scoped(
        &db,
        created.id,
        board::NoteUpdate {
            title: PatchField::Value("Stolen".to_owned()),
            body: PatchField::Missing,
            owner_id: PatchField::Missing,
        },
        &foreign,
    )
    .await;
    assert!(hidden.is_err());
    let empty = NoteSearchForm::from_form(&FormValues::default()).unwrap_err();
    assert_eq!(empty.form_error(), Some("validation.required"));
    let mut search = FormValues::parse_urlencoded_multi("tags=open&tags=done", &["tags"]).unwrap();
    let found = NoteSearchForm::from_form(&search).unwrap();
    assert_eq!(found.tags, ["open", "done"]);
    search.insert("tags", "nope");
    assert_eq!(
        NoteSearchForm::from_form(&search)
            .unwrap_err()
            .for_field("tags"),
        Some("validation.choice")
    );
}

#[tokio::test]
async fn sqlite_note_forms_keep_owner_outside_the_allow_list() {
    let db = fexu::db::connect("sqlite::memory:").await.expect("sqlite");
    publish_and_revise(db).await;
}

#[tokio::test]
#[ignore = "requires DATABASE_URL, CREATEDB, and an approved PostgreSQL test database"]
async fn postgres_note_forms_keep_owner_outside_the_allow_list() {
    let admin = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let disposable = fexu::db::DisposablePostgres::create(&admin)
        .await
        .expect("isolated postgres");
    let db = fexu::db::connect(disposable.url()).await.expect("postgres");
    publish_and_revise(db.clone()).await;
    db.close().await.expect("close");
    disposable.drop_database().await.expect("drop");
}
