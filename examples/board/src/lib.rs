//! A small HTML notes board. CRUD is one resource declaration plus a policy.
//!
//! The handlers, forms, CSRF checks and queries come from `fexu::views::crud_urls_with`.
//! This app does not write SQL.

use std::path::PathBuf;

use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use fexu::apps::{AppConfig, AppError, FexuApplication, Registration, TemplateMode};
use fexu::auth::{AuthUser, ObjectPolicy};
use fexu::core::{FexuModel, QueryScope, ScopeValue};
use fexu::db::driver::DatabaseConnection;
use fexu::prelude::*;
use fexu::serde::{Deserialize, Serialize};
use fexu::serde_json::{Map, Value};
use fexu::urls::{UrlError, UrlSet};
use fexu::views::{
    conditional_response, crud_urls_with, robots_txt, sitemap_xml, CachePolicy, HtmlResource,
    RecordStamp,
};

mod forms;
mod project;
pub use forms::{
    note_cover_field, note_cover_html, note_form_template, tone_widget, NoteFilterForm,
    NotePublishForm, NoteReviseForm, NoteSearchForm, NoteStampForm, ProjectForm, TaskLineForm,
};
pub use project::{
    create_tables, save_project_with_tasks, Member, MemberCreate, Project, Task, TaskCreate,
};

const ORIGIN: &str = "http://127.0.0.1:3000";

#[derive(Debug, Clone, PartialEq, Eq, FexuModel, Serialize, Deserialize)]
#[fexu(table = "board_notes", permissions = "notes")]
pub struct Note {
    #[field(primary_key, auto)]
    pub id: Uuid,
    #[field(required, min_length = 1, max_length = 80)]
    pub title: String,
    #[field(max_length = 500)]
    pub body: Option<String>,
    pub owner_id: Uuid,
}

pub struct NotePolicy;

impl ObjectPolicy<Note> for NotePolicy {
    fn can_read(user: &AuthUser, object: &Note) -> bool {
        object.owner_id == user.id
    }

    fn can_update(user: &AuthUser, object: &Note) -> bool {
        object.owner_id == user.id
    }

    fn can_delete(user: &AuthUser, object: &Note) -> bool {
        object.owner_id == user.id
    }

    fn read_scope(user: &AuthUser) -> QueryScope {
        QueryScope::Equals {
            column: "owner_id",
            value: ScopeValue::Uuid(user.id),
        }
    }

    fn write_scope(user: &AuthUser) -> QueryScope {
        Self::read_scope(user)
    }

    fn can_create(user: &AuthUser, input: &NoteCreate) -> bool {
        input.owner_id == user.id
    }
}

pub struct NoteStamp;

impl RecordStamp<Note> for NoteStamp {
    fn stamp_create(user: &AuthUser, body: &mut Map<String, Value>) {
        body.insert("owner_id".to_owned(), Value::String(user.id.to_string()));
    }

    fn stamp_update(_user: &AuthUser, _current: &Note, body: &mut Map<String, Value>) {
        body.remove("owner_id");
    }
}

pub struct BoardApp;

impl FexuApplication for BoardApp {
    fn config(&self) -> AppConfig {
        AppConfig::new("board").label("Board").prefix("/board")
    }

    fn contribute(&self, registry: &mut Registration<'_>) -> Result<(), AppError> {
        registry.routes(crud_urls_with::<Note, NotePolicy, NoteStamp>(resource())?)?;
        registry.routes(web_routes()?)?;
        registry.templates(template_dir(), TemplateMode::Isolated)?;
        Ok(())
    }
}

pub fn resource() -> HtmlResource {
    HtmlResource {
        list_name: "board.list",
        create_name: "board.create",
        detail_name: "board.detail",
        update_name: "board.update",
        delete_name: "board.delete",
        list_path: "/items",
        create_path: "/items/new",
        detail_path: "/items/{id}",
        update_path: "/items/{id}/edit",
        delete_path: "/items/{id}/delete",
        list_template: "board/list.html",
        detail_template: "board/detail.html",
        form_template: "board/form.html",
        delete_template: "board/delete.html",
        permission: Some("notes"),
        page_size: 20,
        origin: Some(ORIGIN),
        omit: &["owner_id"],
    }
}

/// Creates the note table from model metadata, then the existing auth and flash tables.
pub async fn prepare_database(db: &DatabaseConnection) -> Result<(), String> {
    use fexu::db::driver::ConnectionTrait;
    let backend = db.get_database_backend();
    let kind = fexu::db::kind_of(backend).map_err(|error| error.to_string())?;
    let sql = Note::schema_sql(kind);
    db.execute(fexu::db::driver::Statement::from_string(backend, sql))
        .await
        .map_err(|error| error.to_string())?;
    create_tables(db).await?;
    fexu::auth::apply_migrations(db)
        .await
        .map_err(|error| error.to_string())?;
    fexu::views::create_flash_table(db)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn web_routes() -> Result<UrlSet, UrlError> {
    UrlSet::new()
        .route("board.robots", "/robots.txt", get(robots))?
        .route("board.sitemap", "/sitemap.xml", get(sitemap))
}

async fn robots(headers: HeaderMap) -> Response {
    match robots_txt(
        ORIGIN,
        &["/board/items"],
        &["/board/items/new"],
        Some("/board/sitemap.xml"),
    ) {
        Ok(body) => conditional_response(
            &headers,
            axum::http::StatusCode::OK,
            "text/plain; charset=utf-8",
            body,
            CachePolicy::Public { seconds: 300 },
        ),
        Err(error) => error.into_response(),
    }
}

async fn sitemap(headers: HeaderMap) -> Response {
    match sitemap_xml(ORIGIN, &["/board/items"]) {
        Ok(body) => conditional_response(
            &headers,
            axum::http::StatusCode::OK,
            "application/xml; charset=utf-8",
            body,
            CachePolicy::Public { seconds: 300 },
        ),
        Err(error) => error.into_response(),
    }
}

fn template_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("templates")
}
