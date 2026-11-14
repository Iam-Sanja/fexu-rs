//! HTML list, detail, create, update and delete.
//!
//! The handlers stay ordinary Axum functions. They call the existing session check,
//! `authorize`, `ObjectPolicy` and `ModelRepository`. They do not build SQL.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Extension, Path, RawQuery, State};
use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::response::Response;
use axum::routing::get;
use fexu_auth::{AppState, AuthUser, ObjectPolicy};
use fexu_core::{ColumnKind, FexuError, FexuModel, QueryScope};
use fexu_db::{query_admin_page, AdminPageRequest, DeleteOutcome, ModelRepository};
use fexu_forms::{
    apply_bounds, apply_contracts, bind_submission, model_fields, model_widget, render_fields,
    BoundField, BoundValue, Field, FormErrors, FormValues, ModelField, SubmitMode, Widget,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Map, Value};
use tera::Context;
use uuid::Uuid;

use crate::apps::TemplateCatalog;
use crate::crud::{page_links, page_window, PageQuery};
use crate::flash::{push_flash, take_flashes, FlashLevel};
use crate::urls::{UrlError, UrlSet};
use crate::views::{fail, from_auth, redirect_to, require_mutation, require_user, ViewError};

const SAVED: &str = "Saved";
const DELETED: &str = "Deleted";
const INVALID: &str = "The form is invalid.";

/// Named routes, local paths and templates for one HTML resource.
///
/// Paths are the app-local paths (`/items`), not the mounted prefix. `Registration::routes`
/// adds the prefix and the route map then reverses the full path.
#[derive(Debug, Clone, Copy)]
pub struct HtmlResource {
    pub list_name: &'static str,
    pub create_name: &'static str,
    pub detail_name: &'static str,
    pub update_name: &'static str,
    pub delete_name: &'static str,
    pub list_path: &'static str,
    pub create_path: &'static str,
    pub detail_path: &'static str,
    pub update_path: &'static str,
    pub delete_path: &'static str,
    pub list_template: &'static str,
    pub detail_template: &'static str,
    pub form_template: &'static str,
    pub delete_template: &'static str,
    /// Same contract as `ModelDescriptor::permission`. `None` denies every caller.
    pub permission: Option<&'static str>,
    pub page_size: u32,
    /// Configured origin without a path. `None` omits the canonical link.
    pub origin: Option<&'static str>,
    /// Columns the form must not accept. A stamp can still set them after binding.
    pub omit: &'static [&'static str],
}

/// Fills trusted values after the form has been checked and before the model input is decoded.
pub trait RecordStamp<M: ModelRepository>: Send + Sync + 'static {
    fn stamp_create(user: &AuthUser, body: &mut Map<String, Value>);
    fn stamp_update(user: &AuthUser, current: &M, body: &mut Map<String, Value>);
}

/// Leaves the decoded form unchanged.
pub struct KeepRecord;

impl<M: ModelRepository> RecordStamp<M> for KeepRecord {
    fn stamp_create(_user: &AuthUser, _body: &mut Map<String, Value>) {}
    fn stamp_update(_user: &AuthUser, _current: &M, _body: &mut Map<String, Value>) {}
}

pub fn crud_urls<M, P>(resource: HtmlResource) -> Result<UrlSet, UrlError>
where
    M: ModelRepository + 'static,
    P: ObjectPolicy<M> + 'static,
{
    crud_urls_with::<M, P, KeepRecord>(resource)
}

pub fn crud_urls_with<M, P, S>(resource: HtmlResource) -> Result<UrlSet, UrlError>
where
    M: ModelRepository + 'static,
    P: ObjectPolicy<M> + 'static,
    S: RecordStamp<M> + 'static,
{
    checked(&resource)?;
    prepare_fields::<M>(true, resource.omit)?;
    prepare_fields::<M>(false, resource.omit)?;
    let list = resource;
    let create = resource;
    let detail = resource;
    let update = resource;
    let delete = resource;
    UrlSet::new()
        .route(
            resource.list_name,
            resource.list_path,
            get(move |state, headers, query, catalog, routes| {
                show_list::<M, P>(list, state, headers, query, catalog, routes)
            }),
        )?
        .route(
            resource.create_name,
            resource.create_path,
            get(move |state, headers, catalog, routes| {
                show_create::<M>(create, state, headers, catalog, routes)
            })
            .post(move |state, headers, catalog, routes, body| {
                post_create::<M, P, S>(create, state, headers, catalog, routes, body)
            }),
        )?
        .route(
            resource.detail_name,
            resource.detail_path,
            get(move |state, headers, path, catalog, routes| {
                show_detail::<M, P>(detail, state, headers, path, catalog, routes)
            }),
        )?
        .route(
            resource.update_name,
            resource.update_path,
            get(move |state, headers, path, catalog, routes| {
                show_update::<M, P>(update, state, headers, path, catalog, routes)
            })
            .post(move |state, headers, path, catalog, routes, body| {
                post_update::<M, P, S>(update, state, headers, path, catalog, routes, body)
            }),
        )?
        .route(
            resource.delete_name,
            resource.delete_path,
            get(move |state, headers, path, catalog, routes| {
                show_delete::<M, P>(delete, state, headers, path, catalog, routes)
            })
            .post(move |state, headers, path, catalog, routes, body| {
                post_delete::<M, P>(delete, state, headers, path, catalog, routes, body)
            }),
        )
}

async fn show_list<M, P>(
    resource: HtmlResource,
    State(state): State<AppState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
    Extension(catalog): Extension<Arc<TemplateCatalog>>,
    Extension(routes): Extension<Arc<crate::urls::RouteMap>>,
) -> Response
where
    M: ModelRepository,
    P: ObjectPolicy<M>,
{
    let user = match actor(
        &state,
        &headers,
        Method::GET,
        "",
        resource.permission,
        "read",
    )
    .await
    {
        Ok(user) => user,
        Err(error) => return error_page(&headers, error),
    };
    let query = match parse_list_query(query, resource.page_size) {
        Ok(query) => query,
        Err(error) => return error_page(&headers, error),
    };
    let scope = scope_for::<M, P>(&user, false);
    let loaded = if matches!(scope, QueryScope::None) {
        Ok((Vec::new(), 0))
    } else {
        load_page::<M>(&state, &scope, query).await
    };
    let (rows, total) = match loaded {
        Ok(page) => page,
        Err(error) => return storage_page(&headers, error),
    };
    let list_path = match named(&routes, resource.list_name, &[]) {
        Ok(path) => path,
        Err(error) => return error_page(&headers, error),
    };
    let size = query.page_size.to_string();
    let links = match page_links(&list_path, query, total, &[("page_size", size.as_str())]) {
        Ok(links) => links,
        Err(error) => {
            return error_page(
                &headers,
                ViewError::new(StatusCode::BAD_REQUEST, error.code, error.detail),
            )
        }
    };
    let listed = match list_rows::<M>(&routes, resource.detail_name, rows) {
        Ok(rows) => rows,
        Err(error) => return error_page(&headers, error),
    };
    let create_url = match named(&routes, resource.create_name, &[]) {
        Ok(path) => path,
        Err(error) => return error_page(&headers, error),
    };
    let mut context = Context::new();
    context.insert("rows", &listed);
    context.insert("total", &total);
    context.insert("page", &links.page);
    context.insert("pages", &links.pages);
    context.insert("prev_url", &links.prev.unwrap_or_default());
    context.insert("next_url", &links.next.unwrap_or_default());
    context.insert("create_url", &create_url);
    finish_page(
        &resource,
        &state,
        &headers,
        &catalog,
        &routes,
        &user,
        resource.list_template,
        &list_path,
        context,
        StatusCode::OK,
    )
    .await
}

async fn show_detail<M, P>(
    resource: HtmlResource,
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Extension(catalog): Extension<Arc<TemplateCatalog>>,
    Extension(routes): Extension<Arc<crate::urls::RouteMap>>,
) -> Response
where
    M: ModelRepository,
    P: ObjectPolicy<M>,
{
    let user = match actor(
        &state,
        &headers,
        Method::GET,
        "",
        resource.permission,
        "read",
    )
    .await
    {
        Ok(user) => user,
        Err(error) => return error_page(&headers, error),
    };
    let record = match load_one::<M, P>(&state, &user, &id, Access::Read).await {
        Ok(record) => record,
        Err(error) => return error_page(&headers, error),
    };
    let value = match serde_json::to_value(&record) {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(error = %error, "record could not be rendered");
            return error_page(&headers, server_error());
        }
    };
    let id = match record_id(&value) {
        Ok(id) => id,
        Err(error) => return error_page(&headers, error),
    };
    let path = match named(&routes, resource.detail_name, &[("id", &id)]) {
        Ok(path) => path,
        Err(error) => return error_page(&headers, error),
    };
    let edit_url = match named(&routes, resource.update_name, &[("id", &id)]) {
        Ok(path) => path,
        Err(error) => return error_page(&headers, error),
    };
    let delete_url = match named(&routes, resource.delete_name, &[("id", &id)]) {
        Ok(path) => path,
        Err(error) => return error_page(&headers, error),
    };
    let mut context = Context::new();
    context.insert("record", &value);
    context.insert("edit_url", &edit_url);
    context.insert("delete_url", &delete_url);
    finish_page(
        &resource,
        &state,
        &headers,
        &catalog,
        &routes,
        &user,
        resource.detail_template,
        &path,
        context,
        StatusCode::OK,
    )
    .await
}

async fn show_create<M>(
    resource: HtmlResource,
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(catalog): Extension<Arc<TemplateCatalog>>,
    Extension(routes): Extension<Arc<crate::urls::RouteMap>>,
) -> Response
where
    M: ModelRepository,
{
    let user = match actor(
        &state,
        &headers,
        Method::GET,
        "",
        resource.permission,
        "create",
    )
    .await
    {
        Ok(user) => user,
        Err(error) => return error_page(&headers, error),
    };
    render_form_page::<M>(
        &resource,
        &state,
        &headers,
        &catalog,
        &routes,
        &user,
        true,
        None,
        &FormValues::default(),
        &FormErrors::default(),
        StatusCode::OK,
    )
    .await
}

async fn post_create<M, P, S>(
    resource: HtmlResource,
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(catalog): Extension<Arc<TemplateCatalog>>,
    Extension(routes): Extension<Arc<crate::urls::RouteMap>>,
    body: Bytes,
) -> Response
where
    M: ModelRepository,
    P: ObjectPolicy<M>,
    S: RecordStamp<M>,
{
    let Some(text) = form_text(&headers, &body) else {
        return error_page(&headers, rejected());
    };
    let user = match actor(
        &state,
        &headers,
        Method::POST,
        posted_token(text),
        resource.permission,
        "create",
    )
    .await
    {
        Ok(user) => user,
        Err(error) => return error_page(&headers, error),
    };
    let values = match FormValues::parse_urlencoded(text) {
        Ok(values) => values,
        Err(errors) => {
            return render_form_page::<M>(
                &resource,
                &state,
                &headers,
                &catalog,
                &routes,
                &user,
                true,
                None,
                &FormValues::default(),
                &errors,
                StatusCode::UNPROCESSABLE_ENTITY,
            )
            .await
        }
    };
    let fields = match prepare_fields::<M>(true, resource.omit) {
        Ok(fields) => fields,
        Err(_) => return error_page(&headers, server_error()),
    };
    let bound = match bind_submission(SubmitMode::Create, &fields, &values, &[], resource.omit) {
        Ok(bound) => bound,
        Err(errors) => {
            return invalid_form::<M>(
                &resource, &state, &headers, &catalog, &routes, &user, true, None, &values, errors,
            )
            .await
        }
    };
    let mut payload = bound_json(&bound);
    S::stamp_create(&user, &mut payload);
    let input = match decode_input::<M::Create>(payload) {
        Ok(input) => input,
        Err(errors) => {
            return invalid_form::<M>(
                &resource, &state, &headers, &catalog, &routes, &user, true, None, &values, errors,
            )
            .await
        }
    };
    if !allows(&user, P::can_create(&user, &input)) {
        return error_page(&headers, forbidden());
    }
    let created = match M::create(&state.db, input).await {
        Ok(created) => created,
        Err(FexuError::Validation(detail)) => {
            let errors = form_error_from_validation(&detail).unwrap_or_else(|| {
                let mut errors = FormErrors::default();
                errors.push("", "validation.invalid_type");
                errors
            });
            return invalid_form::<M>(
                &resource, &state, &headers, &catalog, &routes, &user, true, None, &values, errors,
            )
            .await;
        }
        Err(error) => return storage_page(&headers, error),
    };
    let value = match serde_json::to_value(&created) {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(error = %error, "created record could not be rendered");
            return error_page(&headers, server_error());
        }
    };
    let id = match record_id(&value) {
        Ok(id) => id,
        Err(error) => return error_page(&headers, error),
    };
    if let Err(error) = store_flash(&state, user.session_id, FlashLevel::Success, SAVED).await {
        return error_page(&headers, error);
    }
    match named(&routes, resource.detail_name, &[("id", &id)]) {
        Ok(path) => match redirect_to(&path) {
            Ok(response) => response,
            Err(error) => error_page(&headers, error),
        },
        Err(error) => error_page(&headers, error),
    }
}

async fn show_update<M, P>(
    resource: HtmlResource,
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Extension(catalog): Extension<Arc<TemplateCatalog>>,
    Extension(routes): Extension<Arc<crate::urls::RouteMap>>,
) -> Response
where
    M: ModelRepository,
    P: ObjectPolicy<M>,
{
    let user = match actor(
        &state,
        &headers,
        Method::GET,
        "",
        resource.permission,
        "update",
    )
    .await
    {
        Ok(user) => user,
        Err(error) => return error_page(&headers, error),
    };
    let record = match load_one::<M, P>(&state, &user, &id, Access::Update).await {
        Ok(record) => record,
        Err(error) => return error_page(&headers, error),
    };
    let value = match serde_json::to_value(&record) {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(error = %error, "record could not be rendered");
            return error_page(&headers, server_error());
        }
    };
    let fields = match prepare_fields::<M>(false, resource.omit) {
        Ok(fields) => fields,
        Err(_) => return error_page(&headers, server_error()),
    };
    let values = values_from_record(&value, &fields);
    let id = match record_id(&value) {
        Ok(id) => id,
        Err(error) => return error_page(&headers, error),
    };
    render_form_page::<M>(
        &resource,
        &state,
        &headers,
        &catalog,
        &routes,
        &user,
        false,
        Some(&id),
        &values,
        &FormErrors::default(),
        StatusCode::OK,
    )
    .await
}

async fn post_update<M, P, S>(
    resource: HtmlResource,
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Extension(catalog): Extension<Arc<TemplateCatalog>>,
    Extension(routes): Extension<Arc<crate::urls::RouteMap>>,
    body: Bytes,
) -> Response
where
    M: ModelRepository,
    P: ObjectPolicy<M>,
    S: RecordStamp<M>,
{
    let Some(text) = form_text(&headers, &body) else {
        return error_page(&headers, rejected());
    };
    let user = match actor(
        &state,
        &headers,
        Method::POST,
        posted_token(text),
        resource.permission,
        "update",
    )
    .await
    {
        Ok(user) => user,
        Err(error) => return error_page(&headers, error),
    };
    let current = match load_one::<M, P>(&state, &user, &id, Access::Update).await {
        Ok(record) => record,
        Err(error) => return error_page(&headers, error),
    };
    let stored = match serde_json::to_value(&current) {
        Ok(value) => match record_id(&value) {
            Ok(id) => id,
            Err(error) => return error_page(&headers, error),
        },
        Err(error) => {
            tracing::error!(error = %error, "record could not be rendered");
            return error_page(&headers, server_error());
        }
    };
    let values = match FormValues::parse_urlencoded(text) {
        Ok(values) => values,
        Err(errors) => {
            return render_form_page::<M>(
                &resource,
                &state,
                &headers,
                &catalog,
                &routes,
                &user,
                false,
                Some(&stored),
                &FormValues::default(),
                &errors,
                StatusCode::UNPROCESSABLE_ENTITY,
            )
            .await
        }
    };
    let fields = match prepare_fields::<M>(false, resource.omit) {
        Ok(fields) => fields,
        Err(_) => return error_page(&headers, server_error()),
    };
    let bound = match bind_submission(SubmitMode::Patch, &fields, &values, &[], resource.omit) {
        Ok(bound) => bound,
        Err(errors) => {
            return invalid_form::<M>(
                &resource,
                &state,
                &headers,
                &catalog,
                &routes,
                &user,
                false,
                Some(&stored),
                &values,
                errors,
            )
            .await
        }
    };
    let mut payload = bound_json(&bound);
    S::stamp_update(&user, &current, &mut payload);
    let input = match decode_input::<M::Update>(payload) {
        Ok(input) => input,
        Err(errors) => {
            return invalid_form::<M>(
                &resource,
                &state,
                &headers,
                &catalog,
                &routes,
                &user,
                false,
                Some(&stored),
                &values,
                errors,
            )
            .await
        }
    };
    let stored_id = match Uuid::parse_str(&stored) {
        Ok(id) => id,
        Err(_) => return error_page(&headers, server_error()),
    };
    let scope = scope_for::<M, P>(&user, true);
    let updated = match M::update_scoped(&state.db, stored_id, input, &scope).await {
        Ok(updated) => updated,
        Err(FexuError::NotFound) => return error_page(&headers, missing()),
        Err(FexuError::Validation(detail)) => {
            let errors = form_error_from_validation(&detail).unwrap_or_else(|| {
                let mut errors = FormErrors::default();
                errors.push("", "validation.invalid_type");
                errors
            });
            return invalid_form::<M>(
                &resource,
                &state,
                &headers,
                &catalog,
                &routes,
                &user,
                false,
                Some(&stored),
                &values,
                errors,
            )
            .await;
        }
        Err(error) => return storage_page(&headers, error),
    };
    let value = match serde_json::to_value(&updated) {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(error = %error, "updated record could not be rendered");
            return error_page(&headers, server_error());
        }
    };
    let id = match record_id(&value) {
        Ok(id) => id,
        Err(error) => return error_page(&headers, error),
    };
    if let Err(error) = store_flash(&state, user.session_id, FlashLevel::Success, SAVED).await {
        return error_page(&headers, error);
    }
    match named(&routes, resource.detail_name, &[("id", &id)]) {
        Ok(path) => match redirect_to(&path) {
            Ok(response) => response,
            Err(error) => error_page(&headers, error),
        },
        Err(error) => error_page(&headers, error),
    }
}

async fn show_delete<M, P>(
    resource: HtmlResource,
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Extension(catalog): Extension<Arc<TemplateCatalog>>,
    Extension(routes): Extension<Arc<crate::urls::RouteMap>>,
) -> Response
where
    M: ModelRepository,
    P: ObjectPolicy<M>,
{
    let user = match actor(
        &state,
        &headers,
        Method::GET,
        "",
        resource.permission,
        "delete",
    )
    .await
    {
        Ok(user) => user,
        Err(error) => return error_page(&headers, error),
    };
    let record = match load_one::<M, P>(&state, &user, &id, Access::Delete).await {
        Ok(record) => record,
        Err(error) => return error_page(&headers, error),
    };
    delete_page(
        &resource,
        &state,
        &headers,
        &catalog,
        &routes,
        &user,
        &record,
        StatusCode::OK,
    )
    .await
}

async fn post_delete<M, P>(
    resource: HtmlResource,
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Extension(_catalog): Extension<Arc<TemplateCatalog>>,
    Extension(routes): Extension<Arc<crate::urls::RouteMap>>,
    body: Bytes,
) -> Response
where
    M: ModelRepository,
    P: ObjectPolicy<M>,
{
    let Some(text) = form_text(&headers, &body) else {
        return error_page(&headers, rejected());
    };
    let user = match actor(
        &state,
        &headers,
        Method::POST,
        posted_token(text),
        resource.permission,
        "delete",
    )
    .await
    {
        Ok(user) => user,
        Err(error) => return error_page(&headers, error),
    };
    if FormValues::parse_urlencoded(text).is_err() {
        return error_page(&headers, rejected());
    }
    let record = match load_one::<M, P>(&state, &user, &id, Access::Delete).await {
        Ok(record) => record,
        Err(error) => return error_page(&headers, error),
    };
    let record_id = match serde_json::to_value(&record)
        .ok()
        .as_ref()
        .and_then(|value| value.get("id"))
        .and_then(Value::as_str)
        .and_then(|text| Uuid::parse_str(text).ok())
    {
        Some(id) => id,
        None => return error_page(&headers, server_error()),
    };
    let scope = scope_for::<M, P>(&user, true);
    match M::delete_scoped(&state.db, record_id, &scope).await {
        Ok(DeleteOutcome::Deleted) => {}
        Ok(DeleteOutcome::Missing) | Err(FexuError::NotFound) => {
            return error_page(&headers, missing())
        }
        Err(error) => return storage_page(&headers, error),
    }
    if let Err(error) = store_flash(&state, user.session_id, FlashLevel::Success, DELETED).await {
        return error_page(&headers, error);
    }
    match named(&routes, resource.list_name, &[]) {
        Ok(path) => match redirect_to(&path) {
            Ok(response) => response,
            Err(error) => error_page(&headers, error),
        },
        Err(error) => error_page(&headers, error),
    }
}

#[derive(Clone, Copy)]
enum Access {
    Read,
    Update,
    Delete,
}

async fn load_one<M, P>(
    state: &AppState,
    user: &AuthUser,
    raw_id: &str,
    access: Access,
) -> Result<M, ViewError>
where
    M: ModelRepository,
    P: ObjectPolicy<M>,
{
    let id = Uuid::parse_str(raw_id).map_err(|_| missing())?;
    let write = !matches!(access, Access::Read);
    let scope = scope_for::<M, P>(user, write);
    if matches!(scope, QueryScope::None) {
        return Err(missing());
    }
    let record = match M::get_scoped(&state.db, id, &scope).await {
        Ok(Some(record)) => record,
        Ok(None) | Err(FexuError::NotFound) => return Err(missing()),
        Err(error) => return Err(storage_error(error)),
    };
    let permitted = match access {
        Access::Read => P::can_read(user, &record),
        Access::Update => P::can_update(user, &record),
        Access::Delete => P::can_delete(user, &record),
    };
    if allows(user, permitted) {
        Ok(record)
    } else {
        Err(missing())
    }
}

async fn load_page<M>(
    state: &AppState,
    scope: &QueryScope,
    query: PageQuery,
) -> Result<(Vec<M>, u64), FexuError>
where
    M: ModelRepository,
{
    let (offset, limit) = page_window(query).map_err(|_| {
        FexuError::Validation("page starts at 1 and page_size is from 1 to 100".into())
    })?;
    let page = query_admin_page(
        &state.db,
        M::descriptor().table,
        M::columns(),
        AdminPageRequest {
            search_columns: &[],
            search: None,
            sort_column: "id",
            descending: false,
            equal_filters: &[],
            bounds: &[],
            offset,
            limit,
            scope,
        },
    )
    .await?;
    let total = u64::try_from(page.total).map_err(|_| {
        FexuError::Validation("the page total is outside the supported range".into())
    })?;
    let mut rows = Vec::with_capacity(page.rows.len());
    for row in page.rows {
        rows.push(serde_json::from_value(row).map_err(FexuError::database)?);
    }
    Ok((rows, total))
}

fn list_rows<M: Serialize>(
    routes: &crate::urls::RouteMap,
    detail_name: &str,
    rows: Vec<M>,
) -> Result<Vec<Value>, ViewError> {
    let mut listed = Vec::with_capacity(rows.len());
    for row in rows {
        let mut value = serde_json::to_value(&row).map_err(|error| {
            tracing::error!(error = %error, "list row could not be rendered");
            server_error()
        })?;
        let id = record_id(&value)?;
        let href = named(routes, detail_name, &[("id", &id)])?;
        if let Some(object) = value.as_object_mut() {
            object.insert("href".to_owned(), Value::String(href));
        }
        listed.push(value);
    }
    Ok(listed)
}

#[allow(clippy::too_many_arguments)]
async fn render_form_page<M: FexuModel>(
    resource: &HtmlResource,
    state: &AppState,
    headers: &HeaderMap,
    catalog: &TemplateCatalog,
    routes: &crate::urls::RouteMap,
    user: &AuthUser,
    create: bool,
    record_id: Option<&str>,
    values: &FormValues,
    errors: &FormErrors,
    status: StatusCode,
) -> Response {
    let fields = match prepare_fields::<M>(create, resource.omit) {
        Ok(fields) => fields,
        Err(_) => return error_page(headers, server_error()),
    };
    let action = match if create {
        named(routes, resource.create_name, &[])
    } else {
        named(
            routes,
            resource.update_name,
            &[("id", record_id.unwrap_or(""))],
        )
    } {
        Ok(path) => path,
        Err(error) => return error_page(headers, error),
    };
    let csrf = match csrf_token(state, headers).await {
        Ok(token) => token,
        Err(error) => return error_page(headers, error),
    };
    let locale = locale_of(headers);
    let widgets = widgets(&fields);
    let form_html = render_fields(&action, Some(&csrf), values, errors, &widgets, locale);
    let mut context = Context::new();
    context.insert("form_html", &form_html);
    context.insert("mode", if create { "create" } else { "update" });
    finish_page(
        resource,
        state,
        headers,
        catalog,
        routes,
        user,
        resource.form_template,
        &action,
        context,
        status,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn invalid_form<M: FexuModel>(
    resource: &HtmlResource,
    state: &AppState,
    headers: &HeaderMap,
    catalog: &TemplateCatalog,
    routes: &crate::urls::RouteMap,
    user: &AuthUser,
    create: bool,
    record_id: Option<&str>,
    values: &FormValues,
    errors: FormErrors,
) -> Response {
    if let Err(error) = store_flash(state, user.session_id, FlashLevel::Error, INVALID).await {
        return error_page(headers, error);
    }
    render_form_page::<M>(
        resource,
        state,
        headers,
        catalog,
        routes,
        user,
        create,
        record_id,
        values,
        &errors,
        StatusCode::UNPROCESSABLE_ENTITY,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn delete_page<M: Serialize + FexuModel>(
    resource: &HtmlResource,
    state: &AppState,
    headers: &HeaderMap,
    catalog: &TemplateCatalog,
    routes: &crate::urls::RouteMap,
    user: &AuthUser,
    record: &M,
    status: StatusCode,
) -> Response {
    let value = match serde_json::to_value(record) {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(error = %error, "record could not be rendered");
            return error_page(headers, server_error());
        }
    };
    let id = match record_id(&value) {
        Ok(id) => id,
        Err(error) => return error_page(headers, error),
    };
    let action = match named(routes, resource.delete_name, &[("id", &id)]) {
        Ok(path) => path,
        Err(error) => return error_page(headers, error),
    };
    let csrf = match csrf_token(state, headers).await {
        Ok(token) => token,
        Err(error) => return error_page(headers, error),
    };
    let mut context = Context::new();
    context.insert("action", &action);
    context.insert("csrf", &csrf);
    context.insert("label", &record_label::<M>(&value));
    finish_page(
        resource,
        state,
        headers,
        catalog,
        routes,
        user,
        resource.delete_template,
        &action,
        context,
        status,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn finish_page(
    resource: &HtmlResource,
    state: &AppState,
    headers: &HeaderMap,
    catalog: &TemplateCatalog,
    routes: &crate::urls::RouteMap,
    user: &AuthUser,
    template: &str,
    canonical_path: &str,
    mut context: Context,
    status: StatusCode,
) -> Response {
    let flashes = match read_flashes(state, user.session_id).await {
        Ok(flashes) => flashes,
        Err(error) => return error_page(headers, error),
    };
    context.insert("flashes", &flashes);
    let list_url = match named(routes, resource.list_name, &[]) {
        Ok(path) => path,
        Err(error) => return error_page(headers, error),
    };
    context.insert("list_url", &list_url);
    let canonical = match resource.origin {
        Some(origin) => match crate::seo::canonical_url(origin, canonical_path) {
            Ok(url) => url,
            Err(error) => return error_page(headers, error),
        },
        None => String::new(),
    };
    context.insert("canonical", &canonical);
    match catalog.render(template, &context) {
        Ok(body) => crate::seo::conditional_response(
            headers,
            status,
            "text/html; charset=utf-8",
            body,
            crate::seo::CachePolicy::Private,
        ),
        Err(error) => {
            tracing::warn!(error = %error, code = "view.template", "template rendering failed");
            error_page(headers, server_error())
        }
    }
}

async fn actor(
    state: &AppState,
    headers: &HeaderMap,
    method: Method,
    token: &str,
    permission: Option<&str>,
    action: &str,
) -> Result<AuthUser, ViewError> {
    let user = if method.is_safe() {
        require_user(state.auth.as_ref(), headers).await?
    } else {
        require_mutation(state.auth.as_ref(), headers, token).await?
    };
    let auth = state.auth.as_ref().ok_or_else(|| {
        ViewError::new(
            StatusCode::UNAUTHORIZED,
            "auth.not_configured",
            "authentication is not configured",
        )
    })?;
    auth.authorize(&user, permission, action)
        .await
        .map_err(from_auth)?;
    Ok(user)
}

async fn csrf_token(state: &AppState, headers: &HeaderMap) -> Result<String, ViewError> {
    let auth = state.auth.as_ref().ok_or_else(|| {
        ViewError::new(
            StatusCode::UNAUTHORIZED,
            "auth.not_configured",
            "authentication is not configured",
        )
    })?;
    auth.csrf_from_request(headers).await.map_err(from_auth)
}

fn scope_for<M, P>(user: &AuthUser, write: bool) -> QueryScope
where
    M: ModelRepository,
    P: ObjectPolicy<M>,
{
    if user.is_superuser {
        QueryScope::All
    } else if write {
        P::write_scope(user)
    } else {
        P::read_scope(user)
    }
}

fn allows(user: &AuthUser, permitted: bool) -> bool {
    user.is_superuser || permitted
}

fn prepare_fields<M: FexuModel>(create: bool, omit: &[&str]) -> Result<Vec<ModelField>, UrlError> {
    let columns = M::columns();
    for name in omit {
        if !columns.iter().any(|column| column.name == *name) {
            return Err(UrlError {
                code: "crud.fields",
                detail: format!("Omitted field {name} is not a model column."),
            });
        }
    }
    let names = input_names::<M>(create, omit);
    if create && names.is_empty() {
        return Err(UrlError {
            code: "crud.fields",
            detail: "The model has no writable fields.".into(),
        });
    }
    let admin = M::admin_meta().map(|meta| meta.fields).unwrap_or(&[]);
    let mut fields = model_fields(M::columns(), admin, &names).map_err(|_| UrlError {
        code: "crud.fields",
        detail: "The form fields do not match the model.".into(),
    })?;
    apply_bounds(&mut fields, M::numeric_bounds());
    apply_contracts(&mut fields, M::field_contracts());
    Ok(fields)
}

fn input_names<M: FexuModel>(create: bool, omit: &[&str]) -> Vec<&'static str> {
    let contracts = M::field_contracts();
    let mut names: Vec<&'static str> = if contracts.is_empty() {
        M::columns()
            .iter()
            .filter(|column| !column.primary_key)
            .map(|column| column.name)
            .collect()
    } else {
        contracts
            .iter()
            .filter(|contract| {
                if create {
                    contract.in_create
                } else {
                    contract.in_update
                }
            })
            .map(|contract| contract.column)
            .collect()
    };
    names.retain(|name| !omit.contains(name));
    names
}

fn widgets(fields: &[ModelField]) -> Vec<Field> {
    fields
        .iter()
        .map(|field| {
            let mut widget = model_widget(field, field.name);
            if field.kind == ColumnKind::Text && field.max_length.unwrap_or(0) > 80 {
                widget.widget = Widget::Textarea;
            }
            widget
        })
        .collect()
}

fn bound_json(bound: &[BoundField]) -> Map<String, Value> {
    let mut payload = Map::new();
    for field in bound {
        let value = match &field.value {
            BoundValue::Unchanged => continue,
            BoundValue::Null => Value::Null,
            BoundValue::Text(text) => Value::String(text.clone()),
            BoundValue::Integer(number) => Value::Number((*number).into()),
            BoundValue::Boolean(flag) => Value::Bool(*flag),
        };
        payload.insert(field.name.to_owned(), value);
    }
    payload
}

fn decode_input<T: DeserializeOwned>(payload: Map<String, Value>) -> Result<T, FormErrors> {
    match serde_json::from_value(Value::Object(payload)) {
        Ok(input) => Ok(input),
        Err(error) => {
            tracing::warn!(error = %error, "form payload did not match the model input");
            let mut errors = FormErrors::default();
            errors.push("", "validation.invalid_type");
            Err(errors)
        }
    }
}

fn values_from_record(record: &Value, fields: &[ModelField]) -> FormValues {
    let mut values = FormValues::default();
    let Some(object) = record.as_object() else {
        return values;
    };
    if let Some(id) = object.get("id").and_then(Value::as_str) {
        values.insert("id", id);
    }
    for field in fields {
        let Some(value) = object.get(field.name) else {
            continue;
        };
        let text = match value {
            Value::String(text) => text.clone(),
            Value::Number(number) => number.to_string(),
            Value::Bool(true) => "true".to_owned(),
            Value::Bool(false) => "false".to_owned(),
            Value::Null => String::new(),
            _ => continue,
        };
        values.insert(field.name, text);
    }
    values
}

fn record_label<M: FexuModel>(record: &Value) -> String {
    let name = M::columns().iter().find_map(|column| {
        (column.kind == ColumnKind::Text && !column.primary_key).then_some(column.name)
    });
    if let Some(name) = name {
        if let Some(text) = record.get(name).and_then(Value::as_str) {
            return text.to_owned();
        }
    }
    record
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("record")
        .to_owned()
}

fn record_id(value: &Value) -> Result<String, ViewError> {
    value
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| Uuid::parse_str(id).is_ok())
        .map(str::to_owned)
        .ok_or_else(server_error)
}

fn form_error_from_validation(detail: &str) -> Option<FormErrors> {
    let rest = detail.strip_prefix("field:")?;
    let (field, message) = rest.split_once(':')?;
    if field.is_empty()
        || field.len() > 64
        || !field
            .chars()
            .all(|char| char.is_ascii_alphanumeric() || char == '_')
    {
        return None;
    }
    let code = message.trim().split(':').next()?.trim();
    let mut errors = FormErrors::default();
    errors.push(field, static_code(code));
    Some(errors)
}

fn static_code(code: &str) -> &'static str {
    match code {
        "validation.required" => "validation.required",
        "validation.min_length" => "validation.min_length",
        "validation.max_length" => "validation.max_length",
        "validation.min_value" => "validation.min_value",
        "validation.max_value" => "validation.max_value",
        "validation.email" => "validation.email",
        "validation.slug" => "validation.slug",
        "validation.choice" => "validation.choice",
        "validation.invalid_type" => "validation.invalid_type",
        "validation.readonly" => "validation.readonly",
        "validation.unknown_field" => "validation.unknown_field",
        _ => "validation.invalid_type",
    }
}

fn form_text<'a>(headers: &HeaderMap, body: &'a Bytes) -> Option<&'a str> {
    let mime = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let mime = mime.split(';').next().unwrap_or("").trim();
    if !mime.eq_ignore_ascii_case("application/x-www-form-urlencoded") || body.len() > 65_536 {
        return None;
    }
    std::str::from_utf8(body).ok()
}

fn posted_token(body: &str) -> &str {
    for pair in body.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if key == "csrf_token" {
            return value;
        }
    }
    ""
}

fn parse_list_query(raw: Option<String>, default_size: u32) -> Result<PageQuery, ViewError> {
    let mut page = 1u32;
    let mut page_size = default_size;
    let mut seen_page = false;
    let mut seen_size = false;
    let Some(raw) = raw.filter(|value| !value.is_empty()) else {
        return Ok(PageQuery { page, page_size });
    };
    if raw.len() > 256 || raw.chars().any(|char| char.is_control()) {
        return Err(bad_query());
    }
    for pair in raw.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "page" if !seen_page => {
                seen_page = true;
                page = parse_small(value)?;
            }
            "page_size" if !seen_size => {
                seen_size = true;
                page_size = parse_small(value)?;
            }
            _ => return Err(bad_query()),
        }
    }
    Ok(PageQuery { page, page_size })
}

fn parse_small(value: &str) -> Result<u32, ViewError> {
    if value.is_empty() || value.len() > 5 || !value.chars().all(|char| char.is_ascii_digit()) {
        return Err(bad_query());
    }
    value.parse().map_err(|_| bad_query())
}

fn named(
    routes: &crate::urls::RouteMap,
    name: &str,
    params: &[(&str, &str)],
) -> Result<String, ViewError> {
    routes.path(name, params).map_err(|error| {
        tracing::warn!(code = error.code, "named route could not be reversed");
        server_error()
    })
}

async fn read_flashes(state: &AppState, session_id: Uuid) -> Result<Vec<FlashRow>, ViewError> {
    match take_flashes(&state.db, session_id).await {
        Ok(messages) => Ok(messages
            .into_iter()
            .map(|message| FlashRow {
                level: message.level.as_str(),
                text: message.text,
            })
            .collect()),
        Err(error) => {
            tracing::warn!(error = %error, "flash messages could not be read");
            Err(server_error())
        }
    }
}

async fn store_flash(
    state: &AppState,
    session_id: Uuid,
    level: FlashLevel,
    text: &str,
) -> Result<(), ViewError> {
    push_flash(&state.db, session_id, level, text)
        .await
        .map_err(|error| {
            tracing::warn!(error = %error, "flash message was not stored");
            server_error()
        })
}

#[derive(Serialize)]
struct FlashRow {
    level: &'static str,
    text: String,
}

fn checked(resource: &HtmlResource) -> Result<(), UrlError> {
    let paths = [
        resource.list_path,
        resource.create_path,
        resource.detail_path,
        resource.update_path,
        resource.delete_path,
    ];
    for path in paths {
        if crate::views::normalized_path(path).is_none() && !path.contains("{id}") {
            return Err(UrlError {
                code: "url.path",
                detail: "CRUD paths must be local.".into(),
            });
        }
        if path.contains("..")
            || path.contains('\\')
            || path.contains('?')
            || !path.starts_with('/')
        {
            return Err(UrlError {
                code: "url.path",
                detail: "CRUD paths must be local.".into(),
            });
        }
    }
    for path in [
        resource.detail_path,
        resource.update_path,
        resource.delete_path,
    ] {
        if path.matches("{id}").count() != 1 {
            return Err(UrlError {
                code: "url.path",
                detail: "Detail paths need exactly one {id}.".into(),
            });
        }
    }
    if resource.list_path.contains('{') || resource.create_path.contains('{') {
        return Err(UrlError {
            code: "url.path",
            detail: "List and create paths cannot contain parameters.".into(),
        });
    }
    if paths
        .iter()
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        != paths.len()
    {
        return Err(UrlError {
            code: "url.duplicate",
            detail: "CRUD paths must be distinct.".into(),
        });
    }
    for template in [
        resource.list_template,
        resource.detail_template,
        resource.form_template,
        resource.delete_template,
    ] {
        if template.is_empty()
            || template.contains("..")
            || template.contains('\\')
            || template.starts_with('/')
        {
            return Err(UrlError {
                code: "view.template",
                detail: "Template names must stay inside the app namespace.".into(),
            });
        }
    }
    if !(1..=100).contains(&resource.page_size) {
        return Err(UrlError {
            code: "crud.page",
            detail: "page_size is from 1 to 100.".into(),
        });
    }
    if let Some(origin) = resource.origin {
        crate::seo::require_origin(origin).map_err(|error| UrlError {
            code: error.code,
            detail: error.detail,
        })?;
    }
    for name in resource.omit {
        if name.is_empty() || name.len() > 64 {
            return Err(UrlError {
                code: "crud.fields",
                detail: "Omitted field names must be short identifiers.".into(),
            });
        }
    }
    Ok(())
}

fn locale_of(headers: &HeaderMap) -> &'static str {
    headers
        .get(header::ACCEPT_LANGUAGE)
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.to_ascii_lowercase().starts_with("en"))
        .map(|_| "en")
        .unwrap_or("de")
}

fn error_page(headers: &HeaderMap, error: ViewError) -> Response {
    fail(
        headers,
        None,
        error.status,
        error.code,
        &error.detail,
        false,
    )
}

fn storage_page(headers: &HeaderMap, error: FexuError) -> Response {
    error_page(headers, storage_error(error))
}

fn storage_error(error: FexuError) -> ViewError {
    match error {
        FexuError::NotFound => missing(),
        FexuError::Validation(detail) => {
            ViewError::new(StatusCode::UNPROCESSABLE_ENTITY, "validation.field", detail)
        }
        FexuError::Forbidden | FexuError::Denied(_) => forbidden(),
        FexuError::Unauthorized(message) => {
            ViewError::new(StatusCode::UNAUTHORIZED, "auth.required", message)
        }
        FexuError::Database(error) => {
            tracing::error!(error = %error, "web view database operation failed");
            server_error()
        }
    }
}

fn missing() -> ViewError {
    ViewError::new(
        StatusCode::NOT_FOUND,
        "resource.not_found",
        "Resource not found",
    )
}

fn forbidden() -> ViewError {
    ViewError::new(StatusCode::FORBIDDEN, "auth.forbidden", "Access denied")
}

fn rejected() -> ViewError {
    ViewError::new(
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "request.rejected",
        "Request rejected",
    )
}

fn bad_query() -> ViewError {
    ViewError::new(
        StatusCode::BAD_REQUEST,
        "crud.page",
        "The page query is invalid.",
    )
}

fn server_error() -> ViewError {
    ViewError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "server.error",
        "Internal server error",
    )
}
